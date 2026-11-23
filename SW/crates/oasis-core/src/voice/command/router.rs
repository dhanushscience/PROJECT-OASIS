//! Tier A of spoken commands: a deterministic router from a transcript to one button action,
//! against a closed vocabulary. Port of `Services/Voice/VoiceCommandRouter.cs`.
//!
//! It never invents an action. A match is a button the user made, a program name, or one of a
//! few fixed phrases ("type …", "open …", "press …", "switch to …"). Whether the result may run
//! without asking is decided afterwards by [`super::safety`].

use std::collections::HashSet;

use crate::actions::{ActionKind, ButtonAction};
use crate::matching::similarity;
use crate::shortcut::Chord;
use crate::voice::VoiceActionPolicy;

use super::apps::{guess_app_from_text_if, parse_browser_action, parse_browser_open};
use super::lexicon::{LexiconEntry, LexiconKind, RunningApp};
use super::text::normalize;

/// The voice settings the router consults.
#[derive(Debug, Clone, PartialEq)]
pub struct RouterSettings {
    /// How close a lexicon match must be (0 to 1).
    pub match_threshold: f64,
    pub allow_launch_app: bool,
    pub allow_type_text: bool,
    /// The clock, for "in 10 minutes" and "at 7 pm". `None` turns alarm, timer and reminder phrases off.
    pub now: Option<super::reminders::LocalTime>,
    /// Quick Launch setups that can be opened by saying their name.
    pub quick_launch: Vec<crate::quick_launch::QuickSetupRef>,
    /// Spotify is the program running, so "next" and "play X" mean Spotify.
    pub spotify_running: bool,
    /// The programs with a window open now, for "close X".
    pub running: Vec<RunningApp>,
    /// Launcher names of the apps installed, for mishearings that are also words ("cloud").
    /// `None` = not checked (every alias counts).
    pub installed: Option<Vec<String>>,
    /// "copy", "undo", "ctrl z" said on their own press their keys ([`super::edit`]).
    pub edit_commands: bool,
    /// The files and folders on the Desktop and in open Explorer windows, for "open project".
    pub on_screen: Vec<super::on_screen::ScreenItem>,
    /// Names of the installed apps (the cached Start list), for "did you mean …?" after a miss.
    pub app_names: Vec<String>,
}

impl RouterSettings {
    fn is_installed(&self, app: &str) -> bool {
        self.installed.as_ref().is_none_or(|list| list.iter().any(|a| a == app))
    }
}

impl Default for RouterSettings {
    fn default() -> RouterSettings {
        RouterSettings { match_threshold: 0.72, allow_launch_app: true, allow_type_text: true, now: None, quick_launch: Vec::new(), spotify_running: false, running: Vec::new(), installed: None, edit_commands: true, on_screen: Vec::new(), app_names: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteResult {
    pub success: bool,
    pub transcript: String,
    pub message: Option<String>,
    pub action: Option<ButtonAction>,
    /// A short name for the action, for the screen.
    pub label: String,
    pub score: f64,
    /// When nothing matched with confidence but one entry came close: that action and its label,
    /// to offer as "did you mean …?". Never run without asking.
    pub suggestion: Option<(ButtonAction, String)>,
}

impl RouteResult {
    fn failed(transcript: &str, message: impl Into<String>) -> RouteResult {
        RouteResult {
            success: false,
            transcript: transcript.to_string(),
            message: Some(message.into()),
            action: None,
            label: String::new(),
            score: 0.0,
            suggestion: None,
        }
    }

    fn ran(transcript: &str, action: ButtonAction, label: impl Into<String>, score: f64) -> RouteResult {
        RouteResult {
            success: true,
            transcript: transcript.to_string(),
            message: None,
            action: Some(action),
            label: label.into(),
            score,
            suggestion: None,
        }
    }
}

fn action(kind: &str, value: impl Into<String>) -> ButtonAction {
    ButtonAction { kind: kind.into(), value: value.into(), steps: vec![] }
}

/// "open chrome then ctrl t": each part is routed alone and the plan runs only if every part
/// routes. The steps are joined into one automation, so the safety tier of the plan is the
/// highest of its parts (`safety::evaluate` looks at every step). Typed and searched text keeps
/// its "then" as content.
pub fn route(transcript: &str, lexicon: &[LexiconEntry], settings: &RouterSettings) -> RouteResult {
    let normalized = normalize(transcript);
    let spaced = format!(" {normalized} ");
    if !spaced.contains(" then ") || try_type(&normalized, transcript, settings).is_some() || try_search(&normalized, transcript, settings).is_some() {
        return route_one(transcript, lexicon, settings);
    }
    let parts: Vec<&str> = normalized.split(" and then ").flat_map(|p| p.split(" then ")).map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return route_one(transcript, lexicon, settings);
    }
    // A part like "ctrl t" is a shortcut here; said alone it is too short to trust.
    let part = |p: &&str| {
        let r = route_one(p, lexicon, settings);
        let chordy = ["ctrl ", "control ", "alt ", "shift "].iter().any(|m| p.starts_with(m));
        if !r.success && chordy { route_one(&format!("press {p}"), lexicon, settings) } else { r }
    };
    let results: Vec<RouteResult> = parts.iter().map(part).collect();
    if results.iter().all(|r| !r.success) {
        return route_one(transcript, lexicon, settings);
    }
    if let Some((i, r)) = results.iter().enumerate().find(|(_, r)| !r.success) {
        let why = r.message.clone().unwrap_or_default();
        return RouteResult::failed(transcript, format!("Part {} \"{}\" did not match, so nothing ran. {why}", i + 1, parts[i]));
    }
    // Reminders and Spotify are run by the engine as a whole action, never as a step, so a plan
    // holding one would fail halfway: refuse it up front instead.
    let alone = |r: &RouteResult| r.action.as_ref().is_some_and(|a| {
        matches!(a.kind(), ActionKind::Reminder | ActionKind::CancelReminders) || ActionKind::is_spotify_text(&a.kind)
    });
    if let Some(i) = results.iter().position(alone) {
        return RouteResult::failed(transcript, format!("Part {} \"{}\" has to be said on its own, so nothing ran.", i + 1, parts[i]));
    }
    let mut steps = Vec::new();
    for r in &results {
        if !steps.is_empty() {
            steps.push(crate::actions::ActionStep { kind: ActionKind::Delay.as_str().into(), value: "500".into(), ..Default::default() });
        }
        for (kind, value) in super::safety::steps_of(r.action.as_ref().expect("routed")) {
            steps.push(crate::actions::ActionStep { kind, value, ..Default::default() });
        }
    }
    let label = results.iter().map(|r| r.label.as_str()).collect::<Vec<_>>().join(" THEN ");
    let score = results.iter().map(|r| r.score).fold(1.0, f64::min);
    RouteResult::ran(transcript, ButtonAction { kind: ActionKind::Automation.as_str().into(), value: String::new(), steps }, label, score)
}

fn route_one(transcript: &str, lexicon: &[LexiconEntry], settings: &RouterSettings) -> RouteResult {
    let normalized = normalize(transcript);
    if normalized.is_empty() {
        return RouteResult::failed(transcript, "Empty transcript.");
    }

    if let Some(id) = crate::quick_launch::matching(transcript, &settings.quick_launch) {
        return RouteResult::ran(transcript, action(ActionKind::QuickLaunch.as_str(), id), "QUICK LAUNCH", 1.0);
    }
    // A button whose label is exactly what was said beats the built-in phrases below ("lock screen").
    if let Some(entry) = lexicon.iter().find(|e| e.kind == LexiconKind::ButtonLabel && e.normalized == normalized) {
        if let Ok(resolved) = resolve_entry(entry, settings) {
            return RouteResult::ran(transcript, resolved, entry_label(entry), 1.0);
        }
    }
    // Before "paste …" typing, which would paste the word "it" for "paste it".
    if let Some((keys, label)) = super::edit::lookup(&normalized).filter(|_| settings.edit_commands) {
        return RouteResult::ran(transcript, keys, label, 1.0);
    }
    if let Some(result) = try_type(&normalized, transcript, settings) {
        return result;
    }
    if let Some(result) = try_search(&normalized, transcript, settings) {
        return result;
    }
    if let Some(arranged) = super::window_cmd::arrangements(&normalized) {
        return arrange(transcript, &arranged, settings);
    }
    if let Some((kind, value, label)) = super::window_cmd::plan(&normalized) {
        return RouteResult::ran(transcript, action(kind, value), label, 1.0);
    }
    if let Some((volume, label)) = super::volume::plan(&normalized) {
        return RouteResult::ran(transcript, volume, label, 1.0);
    }
    if let Some((kind, value, label)) = settings.now.and_then(|now| super::reminders::plan(&normalized, now)) {
        return RouteResult::ran(transcript, action(kind, value), label, 1.0);
    }
    if let Some((kind, value, label)) = super::system::plan(&normalized) {
        return RouteResult::ran(transcript, action(kind, value), label, 1.0);
    }
    if is_join_meeting(&normalized) {
        return RouteResult::ran(transcript, action(ActionKind::JoinMeeting.as_str(), ""), "JOIN MEETING", 1.0);
    }
    match super::spotify::parse(&normalized, settings.spotify_running) {
        Some(super::spotify::Phrase::Control { command, label }) => {
            return RouteResult::ran(transcript, action(ActionKind::SpotifyControl.as_str(), command), label.to_uppercase(), 1.0);
        }
        Some(super::spotify::Phrase::Play { query }) => {
            let label = format!("PLAY {}", query.to_uppercase());
            return RouteResult::ran(transcript, action(ActionKind::SpotifyPlay.as_str(), query), label, 1.0);
        }
        None => {}
    }
    if let Some((value, label)) = super::system::radio(&normalized) {
        return RouteResult::ran(transcript, action(ActionKind::Radio.as_str(), value), label, 1.0);
    }
    if let Some((value, label)) = super::system::brightness(&normalized) {
        return RouteResult::ran(transcript, action(ActionKind::Brightness.as_str(), value), label, 1.0);
    }
    if let Some((uri, name)) = settings_page(&normalized) {
        return RouteResult::ran(transcript, action(ActionKind::OpenSettings.as_str(), uri), format!("{name} SETTINGS").to_uppercase(), 1.0);
    }
    if let Some(result) = try_open_selected(&normalized, transcript, settings) {
        return result;
    }
    // Before apps and "go to" switching: This PC, Recycle Bin, … are places, not programs. A file
    // or folder on screen with exactly that name wins: "open bin" in a window showing BIN.
    if let Some(item) = shown_by_place_words(&normalized, &settings.on_screen) {
        if !settings.allow_launch_app {
            return RouteResult::failed(transcript, "LaunchApp is disabled for voice.");
        }
        return RouteResult::ran(transcript, action(ActionKind::LaunchApp.as_str(), item.launch_value()), item.name.to_uppercase(), 1.0);
    }
    if let Some((name, address)) = place(&normalized) {
        if !settings.allow_launch_app {
            return RouteResult::failed(transcript, "LaunchApp is disabled for voice.");
        }
        return RouteResult::ran(transcript, action(ActionKind::LaunchApp.as_str(), address), name, 1.0);
    }
    if let Some((browser, chord)) = parse_browser_action(&normalized) {
        use crate::actions::ActionStep;
        let step = |kind: &str, value: &str| ActionStep { kind: kind.into(), value: value.into(), ..Default::default() };
        let label = format!("{chord}{}", browser.as_deref().map(|b| format!(" IN {}", b.to_uppercase())).unwrap_or_default());
        let Some(browser) = browser else { return RouteResult::ran(transcript, action(ActionKind::Shortcut.as_str(), chord), label, 1.0) };
        if !settings.allow_launch_app {
            return RouteResult::failed(transcript, "LaunchApp is disabled for voice.");
        }
        let steps = vec![step(ActionKind::LaunchApp.as_str(), &browser), step(ActionKind::Delay.as_str(), "500"), step(ActionKind::Shortcut.as_str(), chord)];
        return RouteResult::ran(transcript, ButtonAction { kind: ActionKind::Automation.as_str().into(), value: String::new(), steps }, label, 1.0);
    }
    if let Some(result) = try_close(&normalized, transcript, &settings.running) {
        return result;
    }
    if let Some(result) = try_open(&normalized, transcript, settings) {
        return result;
    }
    if let Some(chord) = try_shortcut_phrase(&normalized) {
        return RouteResult::ran(transcript, action(ActionKind::Shortcut.as_str(), chord.clone()), chord, 1.0);
    }
    if let Some(result) = try_switch(&normalized, transcript, lexicon) {
        return result;
    }

    let (best, runner_up) = find_best(&normalized, lexicon);
    let Some((entry, score)) = best else {
        return RouteResult::failed(transcript, format!("Heard \"{transcript}\": no match."));
    };
    if score < settings.match_threshold || score - runner_up < 0.08 {
        let mut result = RouteResult::failed(transcript, format!("Heard \"{transcript}\": no confident match."));
        result.score = score;
        // Close, and clearly ahead of the rest: worth asking about.
        if score >= SUGGEST_FROM && score - runner_up >= 0.08 {
            result.suggestion = resolve_entry(entry, settings).ok().map(|action| (action, entry_label(entry)));
        }
        return result;
    }
    match resolve_entry(entry, settings) {
        Ok(resolved) => RouteResult::ran(transcript, resolved, entry_label(entry), score),
        Err(message) => {
            let mut result = RouteResult::failed(transcript, message);
            result.score = score;
            result
        }
    }
}

/// A near miss at least this close is offered as "did you mean …?" instead of dropped.
const SUGGEST_FROM: f64 = 0.55;

/// "close whatsapp", "close the browser": closes the program's windows, as its own close button
/// would (it may still ask to save). Only a program that is open now; anything else is no match.
fn try_close(normalized: &str, transcript: &str, running: &[RunningApp]) -> Option<RouteResult> {
    let target = normalized.strip_prefix("close ")?.trim();
    let target = target.strip_prefix("the ").unwrap_or(target);
    if target.is_empty() || matches!(target, "parenthesis" | "bracket" | "brackets" | "quote" | "quotes") {
        return None;
    }
    match find_running(target, running) {
        Some(app) => Some(RouteResult::ran(transcript, action(ActionKind::CloseApp.as_str(), &app.process), format!("CLOSE {}", app.process.to_uppercase()), 1.0)),
        None => Some(RouteResult::failed(transcript, format!("\"{target}\" is not open."))),
    }
}

/// The running program `spoken` names: by its program name, a well-known other name ("edge" is
/// `msedge`, "vs code" is `Code`), the start of its window title, or a close spelling.
pub fn find_running<'a>(spoken: &str, running: &'a [RunningApp]) -> Option<&'a RunningApp> {
    use crate::matching::web_catalog::compact;
    let program = |a: &RunningApp| compact(a.process.trim_end_matches(".exe").trim_end_matches(".EXE"));
    let wanted = compact(spoken);
    if wanted.is_empty() {
        return None;
    }
    if spoken == "browser" {
        let browsers = ["chrome", "msedge", "firefox", "brave", "opera", "vivaldi"];
        return running.iter().find(|a| browsers.contains(&program(a).as_str()));
    }
    // "Close plot" / "Close God" are how "close Claude" has been heard; only a running Claude answers.
    let canonical = if matches!(spoken, "plot" | "god") { Some("claude") } else { super::apps::canonical_app(spoken) };
    running
        .iter()
        .find(|a| program(a) == wanted || (canonical.is_some() && super::apps::canonical_app(&a.process) == canonical))
        .or_else(|| running.iter().find(|a| compact(a.title.split(['-', '—', '|']).next().unwrap_or("")) == wanted))
        // A slip of one letter in five ("whatsap"), never on names too short to tell apart.
        .or_else(|| {
            running.iter().find(|a| {
                let name = program(a);
                name.len() >= 5 && crate::matching::edit_distance::levenshtein(&wanted, &name) * 5 <= name.len()
            })
        })
}

/// RunScript, alone or inside an automation, never runs from voice.
fn is_run_script(action: &ButtonAction) -> bool {
    VoiceActionPolicy::is_forbidden(Some(&action.kind))
        || (ActionKind::of_ignore_case(&action.kind) == ActionKind::Automation
            && action.steps.iter().any(|s| VoiceActionPolicy::is_forbidden(Some(&s.kind))))
}

fn entry_label(entry: &LexiconEntry) -> String {
    if entry.label.is_empty() { entry.phrase.to_uppercase() } else { entry.label.clone() }
}

fn find_best<'a>(normalized: &str, lexicon: &'a [LexiconEntry]) -> (Option<(&'a LexiconEntry, f64)>, f64) {
    let mut best: Option<(&LexiconEntry, f64)> = None;
    let mut runner_up = 0.0f64;
    for entry in lexicon {
        let mut score = score(normalized, &entry.normalized);
        if entry.prefer_active {
            score = (score + 0.05).min(1.0);
        }
        match best {
            Some((_, top)) if score <= top => runner_up = runner_up.max(score),
            _ => {
                if let Some((_, top)) = best {
                    runner_up = top;
                }
                best = Some((entry, score));
            }
        }
    }
    (best.filter(|(_, s)| *s > 0.0), runner_up)
}

fn resolve_entry(entry: &LexiconEntry, settings: &RouterSettings) -> Result<ButtonAction, String> {
    match entry.kind {
        LexiconKind::ButtonLabel | LexiconKind::BuiltIn => {
            let Some(action) = &entry.action else { return Err("Matched label has no action.".into()) };
            if is_run_script(action) {
                return Err("RunScript is blocked from voice.".into());
            }
            Ok(action.clone())
        }
        LexiconKind::KnownApp | LexiconKind::ProfileName => {
            if !settings.allow_launch_app {
                return Err("LaunchApp is disabled for voice.".into());
            }
            let alias = entry.app_alias.as_deref().or(entry.profile_name.as_deref()).unwrap_or(&entry.phrase);
            Ok(action(ActionKind::LaunchApp.as_str(), alias))
        }
    }
}

/// "open chrome on my primary screen and open notepad on my secondary screen": one automation that
/// starts each app (or brings it forward) and then moves its window to the screen named.
fn arrange(transcript: &str, apps: &[super::window_cmd::Arrangement], settings: &RouterSettings) -> RouteResult {
    use crate::actions::ActionStep;
    if apps.iter().any(|a| a.open) && !settings.allow_launch_app {
        return RouteResult::failed(transcript, "LaunchApp is disabled for voice.");
    }
    let mut steps = Vec::new();
    for app in apps {
        if app.open {
            let value = guess_app_from_text_if(&app.app, &|a| settings.is_installed(a)).unwrap_or_else(|| app.app.clone());
            steps.push(ActionStep { kind: ActionKind::LaunchApp.as_str().into(), value, ..Default::default() });
        }
        steps.push(ActionStep { kind: ActionKind::WindowCommand.as_str().into(), value: format!("{}|{}", app.screen, app.app), ..Default::default() });
    }
    let label = apps.iter().map(|a| a.app.to_uppercase()).collect::<Vec<_>>().join(" + ");
    RouteResult::ran(transcript, ButtonAction { kind: ActionKind::Automation.as_str().into(), value: String::new(), steps }, format!("ARRANGE {label}"), 1.0)
}

fn try_type(normalized: &str, transcript: &str, settings: &RouterSettings) -> Option<RouteResult> {
    let prefix = ["type ", "paste ", "write "].into_iter().find(|p| normalized.starts_with(p))?;
    let text = normalized[prefix.len()..].trim();
    if text.is_empty() {
        return Some(RouteResult::failed(transcript, "Nothing to type."));
    }
    if !settings.allow_type_text {
        return Some(RouteResult::failed(transcript, "Type/paste is disabled for voice."));
    }
    Some(RouteResult::ran(transcript, action(ActionKind::PasteText.as_str(), text), "TYPE", 1.0))
}

/// "search for X [on site] [in browser]": opens the results page. Before `try_open`, which would read
/// "open chrome and search for X" as launching a program.
fn try_search(normalized: &str, transcript: &str, settings: &RouterSettings) -> Option<RouteResult> {
    let (kind, value, reply) = super::web_search::plan(normalized)?;
    if ActionKind::of(kind) == ActionKind::LaunchApp && !settings.allow_launch_app {
        return Some(RouteResult::failed(transcript, "LaunchApp is disabled for voice."));
    }
    Some(RouteResult::ran(transcript, action(kind, value), reply, 1.0))
}

/// "open bluetooth settings", "show the display settings page", "open settings".
fn settings_page(normalized: &str) -> Option<(&'static str, String)> {
    let rest = ["open ", "show ", "go to ", "take me to ", "launch ", "bring up ", "pull up "]
        .into_iter()
        .find_map(|p| normalized.strip_prefix(p))?;
    let rest = ["the ", "my ", "windows "].into_iter().fold(rest, |r, p| r.strip_prefix(p).unwrap_or(r));
    let name = ["settings page", "settings app", "settings", "panel"]
        .into_iter()
        .find_map(|s| rest.strip_suffix(s))
        .map(str::trim)?;
    let name = if name.is_empty() { "settings" } else { name };
    crate::actions::settings_pages::resolve(name).map(|uri| (uri, name.to_string()))
}

/// "open recycle bin", "show me the trash", "go to my computer": a listed Windows place.
fn place(normalized: &str) -> Option<(&'static str, &'static str)> {
    let rest = PLACE_LEADS.into_iter().find_map(|p| normalized.strip_prefix(p))?;
    crate::actions::places::resolve(rest)
}

/// "open this", "open it", "open these": what is selected in the File Explorer window in front, or
/// on the Desktop. Several selected open together (at most 5).
fn try_open_selected(normalized: &str, transcript: &str, settings: &RouterSettings) -> Option<RouteResult> {
    let rest = ["open up ", "open "].into_iter().find_map(|p| normalized.strip_prefix(p))?;
    const THIS: [&str; 14] = [
        "this", "it", "that", "these", "them", "this one", "that one", "selected", "the selected", "the selected one",
        "this file", "this folder", "the selected file", "the selected folder",
    ];
    if !THIS.contains(&rest) {
        return None;
    }
    if !settings.allow_launch_app {
        return Some(RouteResult::failed(transcript, "LaunchApp is disabled for voice."));
    }
    let picked: Vec<&super::on_screen::ScreenItem> = settings.on_screen.iter().filter(|i| i.selected).collect();
    let launch = |i: &super::on_screen::ScreenItem| action(ActionKind::LaunchApp.as_str(), i.launch_value());
    Some(match picked.as_slice() {
        [] => RouteResult::failed(transcript, "Nothing is selected. Click a file or folder first, or say its name."),
        [one] => RouteResult::ran(transcript, launch(one), one.name.to_uppercase(), 1.0),
        many if many.len() <= 5 => {
            let steps = many.iter().map(|i| crate::actions::ActionStep { kind: ActionKind::LaunchApp.as_str().into(), value: i.launch_value(), ..Default::default() }).collect();
            RouteResult::ran(transcript, ButtonAction { kind: ActionKind::Automation.as_str().into(), value: String::new(), steps }, format!("OPEN {} ITEMS", many.len()), 1.0)
        }
        many => RouteResult::failed(transcript, format!("{} items are selected; select at most 5 to open them by voice.", many.len())),
    })
}

const PLACE_LEADS: [&str; 9] = ["open up ", "open ", "show me ", "show ", "go to ", "take me to ", "launch ", "bring up ", "pull up "];

/// A place phrase ("open the bin folder") whose name is, word for word, a file or folder on screen.
fn shown_by_place_words<'a>(normalized: &str, on_screen: &'a [super::on_screen::ScreenItem]) -> Option<&'a super::on_screen::ScreenItem> {
    place(normalized)?;
    let rest = PLACE_LEADS.into_iter().find_map(|p| normalized.strip_prefix(p))?;
    let rest = ["the ", "my "].into_iter().find_map(|p| rest.strip_prefix(p)).unwrap_or(rest);
    let rest = [" folder", " file", " window"].into_iter().find_map(|s| rest.strip_suffix(s)).unwrap_or(rest);
    match super::on_screen::find(rest, on_screen) {
        super::on_screen::Found::Exact(item) => Some(item),
        _ => None,
    }
}

/// "join my meeting", "join the next call", "join meeting now".
fn is_join_meeting(normalized: &str) -> bool {
    let Some(rest) = normalized.strip_prefix("join ") else { return false };
    let rest = rest.strip_suffix(" now").unwrap_or(rest);
    let rest = ["the ", "my ", "our "].into_iter().fold(rest, |r, p| r.strip_prefix(p).unwrap_or(r));
    let rest = ["next ", "current ", "upcoming "].into_iter().fold(rest, |r, p| r.strip_prefix(p).unwrap_or(r));
    matches!(rest, "meeting" | "call" | "video call" | "calendar meeting" | "google meet" | "meet")
}

fn try_open(normalized: &str, transcript: &str, settings: &RouterSettings) -> Option<RouteResult> {
    let prefix = ["open ", "launch ", "start ", "run "].into_iter().find(|p| normalized.starts_with(p))?;
    let target = normalized[prefix.len()..].trim();
    if target.is_empty() {
        return Some(RouteResult::failed(transcript, "Nothing to open."));
    }
    if !settings.allow_launch_app {
        return Some(RouteResult::failed(transcript, "LaunchApp is disabled for voice."));
    }
    if let Some((browser, url)) = parse_browser_open(normalized) {
        let label = format!("OPEN {url} IN {browser}").to_uppercase();
        return Some(RouteResult::ran(transcript, action(ActionKind::LaunchApp.as_str(), format!("{browser}|{url}")), label, 1.0));
    }
    // "open the chrome window", "open notepad app": the app is the name between the filler words.
    let target = target.strip_prefix("the ").unwrap_or(target);
    let target = ["window", "app", "application"].into_iter().find_map(|w| target.strip_suffix(w).and_then(|t| t.strip_suffix(' '))).filter(|t| !t.is_empty()).unwrap_or(target);
    // What is on screen: its exact name beats an app, a near name only when no app is meant.
    let shown = target.strip_suffix(" folder").or_else(|| target.strip_suffix(" file")).unwrap_or(target);
    let open_item = |item: &super::on_screen::ScreenItem| RouteResult::ran(transcript, action(ActionKind::LaunchApp.as_str(), item.launch_value()), item.name.to_uppercase(), 1.0);
    let app = guess_app_from_text_if(target, &|a| settings.is_installed(a));
    // "notepad" is the app, not the "Notepad++" shortcut: against an app, the name must match
    // letter for letter, symbols included.
    let same_name = |item: &super::on_screen::ScreenItem| app.is_none() || item.stem().to_lowercase() == shown;
    match super::on_screen::find(shown, &settings.on_screen) {
        super::on_screen::Found::Exact(item) if same_name(item) => return Some(open_item(item)),
        super::on_screen::Found::Close(item) if app.is_none() => return Some(open_item(item)),
        super::on_screen::Found::Many(items) if app.is_none() => {
            let names = items.iter().take(4).map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ");
            return Some(RouteResult::failed(transcript, format!("\"{shown}\" could be any of: {names}. Say more of the name.")));
        }
        _ => {}
    }
    let known = app.is_some();
    let guess = app.unwrap_or_else(|| target.to_string());
    let mut result = RouteResult::ran(transcript, action(ActionKind::LaunchApp.as_str(), guess), target.to_uppercase(), 1.0);
    // Used only if no such app turns out to be on the PC (decide.rs asks "Did you mean …?").
    if !known {
        result.suggestion = nearest(shown, settings);
    }
    Some(result)
}

/// The one thing "open X" most likely meant when X itself is not there: the closest name among the
/// Windows places, the Settings pages, what is on screen and the installed apps. Only a clear
/// winner (close enough, and ahead of the next different thing) is offered; it never runs unasked.
fn nearest(spoken: &str, settings: &RouterSettings) -> Option<(ButtonAction, String)> {
    if crate::matching::web_catalog::compact(spoken).len() < 3 {
        return None;
    }
    let launch = |value: String| action(ActionKind::LaunchApp.as_str(), value);
    let mut candidates: Vec<(String, ButtonAction, String)> = Vec::new();
    for (said, name, address) in crate::actions::places::names() {
        candidates.push((said.to_string(), launch(address.to_string()), name.to_string()));
    }
    for (said, uri) in crate::actions::settings_pages::names() {
        candidates.push((said.to_string(), action(ActionKind::OpenSettings.as_str(), uri), format!("{said} settings").to_uppercase()));
    }
    for item in &settings.on_screen {
        candidates.push((item.stem().to_lowercase(), launch(item.launch_value()), item.name.to_uppercase()));
    }
    for name in super::apps::known_launch_names().map(str::to_string).chain(settings.app_names.iter().cloned()) {
        candidates.push((name.to_lowercase(), launch(name.clone()), name.to_uppercase()));
    }
    let mut best: Option<(f64, &ButtonAction, &String)> = None;
    let mut runner_up = 0.0f64;
    for (said, act, label) in &candidates {
        let s = score(spoken, said);
        match best {
            // The same thing under another name is no rival ("trash" and "recycle bin").
            Some((top, top_act, _)) if top_act == act => {
                if s > top {
                    best = Some((s, act, label));
                }
            }
            Some((top, _, _)) if s <= top => runner_up = runner_up.max(s),
            _ => {
                runner_up = runner_up.max(best.map_or(0.0, |b| b.0));
                best = Some((s, act, label));
            }
        }
    }
    let (top, act, label) = best?;
    (top >= SUGGEST_FROM && top - runner_up >= 0.08).then(|| (act.clone(), label.clone()))
}

fn try_switch(normalized: &str, transcript: &str, lexicon: &[LexiconEntry]) -> Option<RouteResult> {
    let prefix = ["switch to ", "focus ", "go to "].into_iter().find(|p| normalized.starts_with(p))?;
    let target = normalized[prefix.len()..].trim();
    if target.is_empty() {
        return Some(RouteResult::failed(transcript, "Nothing to switch to."));
    }
    let best = lexicon
        .iter()
        .filter(|e| matches!(e.kind, LexiconKind::ProfileName | LexiconKind::KnownApp))
        .map(|e| (e, score(target, &e.normalized)))
        .fold(None::<(&LexiconEntry, f64)>, |top, next| match top {
            Some((_, s)) if s >= next.1 => top,
            _ => Some(next),
        });
    let Some((entry, score)) = best.filter(|(_, s)| *s >= 0.72) else {
        return Some(RouteResult::failed(transcript, format!("No app/profile match for \"{target}\".")));
    };
    let process = entry.app_alias.as_deref().or(entry.profile_name.as_deref()).unwrap_or(&entry.phrase);
    Some(RouteResult::ran(transcript, action(ActionKind::LaunchApp.as_str(), process), format!("SWITCH {process}").to_uppercase(), score.min(0.95)))
}

/// "press control shift t" → `Ctrl+Shift+T`, if that is a real shortcut.
fn try_shortcut_phrase(normalized: &str) -> Option<String> {
    if !["press ", "hotkey ", "shortcut "].iter().any(|p| normalized.starts_with(p)) {
        return None;
    }
    let rest = normalized.replace("press ", "").replace("hotkey ", "").replace("shortcut ", "");
    let chord = spoken_to_chord(rest.trim());
    (!chord.is_empty() && Chord::is_valid(&chord)).then_some(chord)
}

fn spoken_to_chord(spoken: &str) -> String {
    let map = |word: &str| -> Option<&'static str> {
        Some(match word.to_ascii_lowercase().as_str() {
            "control" | "ctrl" => "Ctrl",
            "command" | "win" | "windows" => "Win",
            "option" | "alt" => "Alt",
            "shift" => "Shift",
            "escape" | "esc" => "Esc",
            "enter" | "return" => "Enter",
            "space" => "Space",
            "tab" => "Tab",
            "delete" => "Delete",
            "backspace" => "Backspace",
            _ => return None,
        })
    };
    let mut keys: Vec<String> = Vec::new();
    for part in spoken.split_whitespace() {
        if matches!(part, "and" | "plus" | "+") {
            continue;
        }
        if let Some(mapped) = map(part) {
            keys.push(mapped.to_string());
        } else if part.chars().count() == 1 || part.chars().all(|c| c.is_ascii_digit()) {
            keys.push(part.to_uppercase());
        } else {
            let mut chars = part.chars();
            let first = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
            keys.push(format!("{first}{}", chars.as_str().to_lowercase()));
        }
    }
    keys.join("+")
}

/// How well a spoken phrase matches a vocabulary entry, 0 to 1.
fn score(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    if a.contains(b) || b.contains(a) {
        let (shorter, longer) = (a.len().min(b.len()), a.len().max(b.len()));
        return 0.85 * shorter as f64 / longer as f64;
    }
    let set_a: HashSet<&str> = a.split_whitespace().collect();
    let set_b: HashSet<&str> = b.split_whitespace().collect();
    if !set_a.is_empty() && !set_b.is_empty() {
        let intersection = set_a.intersection(&set_b).count();
        let union = set_a.union(&set_b).count();
        if union > 0 && intersection > 0 {
            let jaccard = intersection as f64 / union as f64;
            if jaccard >= 0.5 {
                return 0.5 + 0.3 * jaccard;
            }
        }
    }
    0.8 * similarity(a, b)
}

#[cfg(test)]
mod tests {
    use super::super::lexicon::{build_lexicon, LabeledAction, LexiconInputs, LexiconProfile};
    use super::*;

    #[test]
    fn a_spoken_search_opens_the_results_not_a_program() {
        let s = RouterSettings::default();
        let r = route("Search for mobile phones.", &[], &s);
        assert_eq!(r.action, Some(action("OpenUrl", "https://www.google.com/search?q=mobile+phones")));
        let r = route("Open Chrome and search for cats", &[], &s);
        assert_eq!(r.action, Some(action("LaunchApp", "chrome|https://www.google.com/search?q=cats")));
        let off = RouterSettings { allow_launch_app: false, ..s };
        assert!(!route("open chrome and search for cats", &[], &off).success);
    }

    #[test]
    fn then_joins_parts_into_one_plan_and_keeps_content_thens() {
        let s = RouterSettings::default();
        let steps = |t: &str| route(t, &[], &s).action.map(|a| a.steps.into_iter().map(|x| (x.kind, x.value)).collect::<Vec<_>>());
        let r = steps("Open Chrome then Ctrl T").expect("compound");
        assert_eq!(r.first().map(|x| x.0.as_str()), Some("LaunchApp"));
        assert_eq!(r.last(), Some(&("Shortcut".to_string(), "Ctrl+T".to_string())));
        let two = steps("Open new tab in Chrome then close tab in chrome").expect("two browser parts");
        assert_eq!(two.iter().filter(|x| x.0 == "Shortcut").count(), 2);
        // One part that does not route: nothing runs, and the part is named.
        let bad = route("open chrome then banana split", &[], &s);
        assert!(!bad.success && bad.action.is_none() && bad.message.unwrap().contains("banana split"));
        // A caution part keeps the whole plan asking.
        let lock = route("open chrome then lock screen", &[], &s);
        let a = lock.action.expect("compound");
        assert!(super::super::safety::evaluate(&super::super::safety::steps_of(&a)).requires_confirmation);
        // Spotify (and reminders) run only as a whole action, never as a step: refused, and named.
        let spotify = route("play lo-fi on spotify then open chrome", &[], &s);
        assert!(!spotify.success && spotify.action.is_none(), "{spotify:?}");
        assert!(spotify.message.unwrap().contains("on its own"));
        // "then" inside a search stays a search.
        let q = route("search for what happened then", &[], &s);
        assert_eq!(q.action, Some(action("OpenUrl", "https://www.google.com/search?q=what+happened+then")));
    }

    fn shortcut(value: &str) -> ButtonAction {
        action("Shortcut", value)
    }

    fn lexicon() -> Vec<LexiconEntry> {
        let active = vec![
            LabeledAction { label: "NEW TAB".into(), action: shortcut("Ctrl+T") },
            LabeledAction { label: "BACK".into(), action: shortcut("Alt+Left") },
        ];
        let profiles = vec![
            LexiconProfile { name: "Chrome".into(), buttons: active.clone() },
            LexiconProfile {
                name: "Scripts".into(),
                buttons: vec![LabeledAction { label: "RUN BACKUP".into(), action: action("RunScript", "backup.ps1") }],
            },
        ];
        build_lexicon(&LexiconInputs { active: Some(("Chrome", &active)), profiles: &profiles, running: &[] })
    }

    fn run(text: &str) -> RouteResult {
        route(text, &lexicon(), &RouterSettings::default())
    }

    #[test]
    fn windows_places_open_however_they_are_asked_for() {
        // The demo miss of 2026-11-19 (docs/OPEN_ANYTHING.md): places are not programs.
        let opens = |said: &str| run(said).action.map(|a| format!("{}:{}", a.kind, a.value));
        let bin = Some("LaunchApp:shell:RecycleBinFolder".to_string());
        for said in ["open recycle bin", "Open the Recycle Bin.", "show me the trash", "open recycling bin", "go to the recycle bin", "open up the bin", "pull up recycle been", "Open recycle then.", "Open recycle bit.", "Open recycle bank."] {
            assert_eq!(opens(said), bin, "{said}");
        }
        let pc = Some("LaunchApp:shell:MyComputerFolder".to_string());
        for said in ["open this pc", "Open This PC.", "go to my computer", "show this computer", "open this pc window"] {
            assert_eq!(opens(said), pc, "{said}");
        }
        assert_eq!(opens("open downloads folder").as_deref(), Some("LaunchApp:shell:Downloads"));
        assert_eq!(opens("open my documents").as_deref(), Some("LaunchApp:shell:Personal"));
        assert_eq!(opens("open control panel").as_deref(), Some("LaunchApp:shell:ControlPanelFolder"));
        assert_eq!(run("open recycle bin").label, "RECYCLE BIN");
        assert_eq!(opens("open file explorer").as_deref(), Some("LaunchApp:explorer"), "File Explorer is still an app");
        let off = RouterSettings { allow_launch_app: false, ..RouterSettings::default() };
        assert!(!route("open recycle bin", &[], &off).success);
    }

    #[test]
    fn a_near_miss_offers_the_closest_thing_and_never_runs_it() {
        use super::super::on_screen::ScreenItem;
        let s = RouterSettings {
            on_screen: vec![ScreenItem { name: "Arduino".into(), path: r"G:\ARDUINO".into(), folder: true, front: true, selected: false }],
            app_names: vec!["Docklight".into(), "OBS Studio".into(), "Spotify".into()],
            ..RouterSettings::default()
        };
        let offer = |said: &str| route(said, &[], &s).suggestion.map(|(a, label)| (a.kind, a.value, label));
        assert_eq!(offer("open recyle bun"), Some(("LaunchApp".into(), "shell:RecycleBinFolder".into(), "RECYCLE BIN".into())));
        assert_eq!(offer("open blutooth"), Some(("OpenSettings".into(), "ms-settings:bluetooth".into(), "BLUETOOTH SETTINGS".into())));
        assert_eq!(route("open arduno", &[], &s).action.map(|a| a.value).as_deref(), Some(r"G:\ARDUINO\"), "a slip of one letter opens at once (stage 6a)");
        assert_eq!(offer("open arduweno"), Some(("LaunchApp".into(), r"G:\ARDUINO\".into(), "ARDUINO".into())), "further off: asked");
        assert_eq!(offer("open doclight"), Some(("LaunchApp".into(), "Docklight".into(), "DOCKLIGHT".into())));
        assert_eq!(offer("open spotifi").map(|o| o.1).as_deref(), Some("Spotify"));
        assert_eq!(offer("open the big blue door"), None, "nothing close: no question");
        assert_eq!(offer("open xy"), None, "too short to guess");
        assert_eq!(offer("open chrome"), None, "a known app needs no question");
        assert!(route("open recyle bun", &[], &s).action.is_some(), "the launch is still tried first; decide.rs asks only when it is not there");
    }

    #[test]
    fn open_this_opens_what_is_selected() {
        use super::super::on_screen::ScreenItem;
        let item = |name: &str, selected| ScreenItem { name: name.into(), path: format!(r"G:\{name}"), folder: true, front: true, selected };
        let with = |items: Vec<ScreenItem>| RouterSettings { on_screen: items, ..RouterSettings::default() };
        let one = with(vec![item("BIN", true), item("BOOKS", false)]);
        for said in ["Open this.", "open it", "open the selected folder", "open that one"] {
            assert_eq!(route(said, &[], &one).action.map(|a| a.value).as_deref(), Some(r"G:\BIN\"), "{said}");
        }
        let two = route("open these", &[], &with(vec![item("BIN", true), item("BOOKS", true)]));
        assert_eq!(two.label, "OPEN 2 ITEMS");
        assert_eq!(two.action.unwrap().steps.len(), 2);
        let none = route("open this", &[], &with(vec![item("BIN", false)]));
        assert!(!none.success && none.message.unwrap().starts_with("Nothing is selected"));
        let lots = route("open these", &[], &with((0..6).map(|i| item(&format!("F{i}"), true)).collect()));
        assert!(!lots.success);
    }

    #[test]
    fn what_is_on_screen_opens_by_name() {
        use super::super::on_screen::ScreenItem;
        let item = |name: &str, folder| ScreenItem { name: name.into(), path: format!(r"C:\Users\me\Desktop\{name}"), folder, front: false, selected: false };
        let s = RouterSettings {
            on_screen: vec![item("Project", true), item("Budget 2026.xlsx", false), item("Chromebook notes", true), item("Old", true), item("Older", true), item("Notepad++.lnk", false)],
            ..RouterSettings::default()
        };
        let opens = |said: &str| route(said, &[], &s).action.map(|a| a.value);
        assert_eq!(opens("open project").as_deref(), Some(r"C:\Users\me\Desktop\Project\"));
        assert_eq!(opens("Open the project folder.").as_deref(), Some(r"C:\Users\me\Desktop\Project\"));
        assert_eq!(opens("open budget").as_deref(), Some(r"C:\Users\me\Desktop\Budget 2026.xlsx"));
        assert_eq!(route("open project", &[], &s).label, "PROJECT");
        assert_eq!(opens("open chrome").as_deref(), Some("chrome"), "an app beats a name that only starts the same");
        assert_eq!(opens("open notepad").as_deref(), Some("notepad"), "the app, not the Notepad++ shortcut (found on the owner's Desktop)");
        assert_eq!(opens("open old").as_deref(), Some(r"C:\Users\me\Desktop\Old\"), "the exact name wins over a longer one");
        assert_eq!(opens("open recycle bin").as_deref(), Some("shell:RecycleBinFolder"), "places come first");
        // The owner's case (2026-11-19): G:\ open in front, with a folder called BIN.
        let g = RouterSettings { on_screen: vec![ScreenItem { name: "BIN".into(), path: r"G:\BIN".into(), folder: true, front: true, selected: false }], ..RouterSettings::default() };
        let value = |said: &str, s: &RouterSettings| route(said, &[], s).action.map(|a| a.value);
        for said in ["open bin", "Open the bin folder.", "go to bin", "show me the bin"] {
            assert_eq!(value(said, &g).as_deref(), Some(r"G:\BIN\"), "{said}");
        }
        assert_eq!(value("open recycle bin", &g).as_deref(), Some("shell:RecycleBinFolder"), "the full place name is still the place");
        assert_eq!(value("open bin", &RouterSettings::default()).as_deref(), Some("shell:RecycleBinFolder"), "no BIN folder on screen");
        let both = RouterSettings { on_screen: vec![item("Project old", true), item("Project new", true)], ..RouterSettings::default() };
        let r = route("open project", &[], &both);
        assert!(!r.success && r.message.unwrap().contains("Project old, Project new"));
    }

    #[test]
    fn saying_a_button_label_runs_that_button() {
        let r = run("new tab");
        assert!(r.success, "{r:?}");
        assert_eq!(r.action, Some(shortcut("Ctrl+T")));
        assert_eq!(r.label, "NEW TAB");
    }

    #[test]
    fn a_slightly_misheard_label_still_matches() {
        let r = run("new tap");
        assert_eq!(r.action, Some(shortcut("Ctrl+T")), "{r:?}");
    }

    #[test]
    fn builtin_phrases_run_media_keys() {
        assert_eq!(run("volume up").action, Some(shortcut("VolumeUp")));
        assert_eq!(run("Hey Oasis, mute").action, Some(shortcut("VolumeMute")));
        assert_eq!(run("next track").action, Some(shortcut("MediaNextTrack")));
    }

    #[test]
    fn a_volume_level_is_routed_and_runs_without_asking() {
        let r = run("set volume to 40 percent");
        assert!(r.success, "{r:?}");
        assert_eq!(r.label, "VOLUME 40%");
        assert_eq!(r.action, Some(action("Volume", "40")));
        let steps = [("Volume".to_string(), "40".to_string())];
        let verdict = super::super::safety::evaluate(&steps);
        assert!(!verdict.blocked && !verdict.requires_confirmation);
    }

    #[test]
    fn locking_asks_first_and_theme_does_not() {
        let lock = run("lock the computer");
        assert_eq!(lock.action, Some(action("LockPc", "lock")));
        assert!(super::super::safety::evaluate(&[("LockPc".into(), "lock".into())]).requires_confirmation);
        assert_eq!(run("switch to dark mode").action, Some(action("Theme", "dark")));
    }

    #[test]
    fn your_own_button_beats_a_built_in_phrase() {
        let buttons = vec![LabeledAction { label: "LOCK SCREEN".into(), action: shortcut("Win+L") }];
        let profiles = vec![LexiconProfile { name: "Mine".into(), buttons: buttons.clone() }];
        let lex = build_lexicon(&LexiconInputs { active: Some(("Mine", &buttons)), profiles: &profiles, running: &[] });
        let r = route("lock screen", &lex, &RouterSettings::default());
        assert_eq!(r.action, Some(shortcut("Win+L")));
        assert_eq!(run("lock screen").action, Some(action("LockPc", "lock")), "without the button the phrase still works");
    }

    #[test]
    fn switching_a_radio_off_asks_and_on_does_not() {
        let ask = |t: &str| {
            let a = run(t).action.unwrap();
            super::super::safety::evaluate(&[(a.kind, a.value)]).requires_confirmation
        };
        assert!(ask("turn off wifi") && ask("turn off bluetooth") && ask("airplane mode on"));
        assert!(!ask("turn on wifi") && !ask("bluetooth on") && !ask("airplane mode off"));
    }

    #[test]
    fn timer_phrases_need_the_clock() {
        let with = RouterSettings { now: Some(super::super::reminders::LocalTime { unix: 1000, offset_secs: 0 }), ..Default::default() };
        let r = route("set a timer for 5 minutes", &lexicon(), &with);
        assert_eq!(r.action, Some(action("Reminder", "timer|1300|")));
        assert_eq!(r.label, "Timer for 5 minutes");
        assert!(!run("set a timer for 5 minutes").success, "without a clock the phrase is not a timer");
    }

    #[test]
    fn spotify_phrases_are_routed_and_bare_words_stay_with_the_media_keys() {
        assert_eq!(run("pause spotify").action, Some(action("SpotifyControl", "pause")));
        assert_eq!(run("play loser on spotify").action, Some(action("SpotifyPlay", "loser")));
        assert_eq!(run("next track").action, Some(shortcut("MediaNextTrack")), "Spotify not running: the media key");
        let running = RouterSettings { spotify_running: true, ..Default::default() };
        assert_eq!(route("next", &lexicon(), &running).action, Some(action("SpotifyControl", "next")));
        assert_eq!(route("play numb", &lexicon(), &running).action, Some(action("SpotifyPlay", "numb")));
    }

    #[test]
    fn join_meeting_phrases() {
        for said in ["join my meeting", "join the next call", "join meeting now", "join the current meeting"] {
            assert_eq!(run(said).action, Some(action("JoinMeeting", "")), "{said}");
        }
        assert!(!matches!(run("join the team").action, Some(ref a) if a.kind == "JoinMeeting"));
    }

    #[test]
    fn brightness_phrases_are_routed() {
        let r = run("set brightness to 60");
        assert_eq!(r.action, Some(action("Brightness", "60")));
        assert_eq!(r.label, "BRIGHTNESS 60%");
    }

    #[test]
    fn open_a_settings_page_by_name() {
        assert_eq!(run("open bluetooth settings").action, Some(action("OpenSettings", "ms-settings:bluetooth")));
        assert_eq!(run("show the night light settings page").action, Some(action("OpenSettings", "ms-settings:nightlight")));
        assert_eq!(run("open settings").action, Some(action("OpenSettings", "ms-settings:")));
        assert_eq!(run("open moon settings").action, Some(action("LaunchApp", "moon settings")), "unlisted pages fall through");
    }

    #[test]
    fn type_and_paste_make_a_paste_action_unless_voice_typing_is_off() {
        let r = run("type hello world");
        assert_eq!(r.action, Some(action("PasteText", "hello world")));
        let off = route("type hello", &lexicon(), &RouterSettings { allow_type_text: false, ..Default::default() });
        assert!(!off.success);
        assert_eq!(off.message.as_deref(), Some("Type/paste is disabled for voice."));
        assert!(!run("type").success, "a bare verb has nothing to type");
    }

    #[test]
    fn open_launches_an_app_by_its_launcher_name() {
        assert_eq!(run("open vs code").action, Some(action("LaunchApp", "code")));
        assert_eq!(run("launch spot i fi").action, Some(action("LaunchApp", "spotify")));
        assert_eq!(run("open my cool tool").action, Some(action("LaunchApp", "my cool tool")), "unknown names are passed on");
        assert!(!run("open").success, "a bare verb has nothing to open");
    }

    #[test]
    fn open_a_site_in_a_browser_makes_a_browser_launch() {
        let r = run("open youtube in brave");
        assert_eq!(r.action, Some(action("LaunchApp", "brave|https://www.youtube.com")));
    }

    #[test]
    fn browser_actions_are_shortcuts_not_site_guesses() {
        let steps = |text: &str| run(text).action.map(|a| a.steps.iter().map(|s| (s.kind.clone(), s.value.clone())).collect::<Vec<_>>());
        let s = |k: &str, v: &str| (k.to_string(), v.to_string());
        let expect = |browser: &str, chord: &str| Some(vec![s("LaunchApp", browser), s("Delay", "500"), s("Shortcut", chord)]);
        assert_eq!(steps("Open new tab in Chrome"), expect("chrome", "Ctrl+T"));
        assert_eq!(steps("open a new window in firefox"), expect("firefox", "Ctrl+N"));
        assert_eq!(steps("open an incognito window in brave"), expect("brave", "Ctrl+Shift+N"));
        assert_eq!(steps("close tab in chrome"), expect("chrome", "Ctrl+W"));
        assert_eq!(run("new tab").action, Some(shortcut("Ctrl+T")), "no browser named: the focused window");
        assert_eq!(run("open youtube in brave").action, Some(action("LaunchApp", "brave|https://www.youtube.com")));
        assert_eq!(run("open github").action, Some(action("LaunchApp", "github")));
    }

    #[test]
    fn open_is_refused_when_voice_may_not_launch_apps() {
        let r = route("open notepad", &lexicon(), &RouterSettings { allow_launch_app: false, ..Default::default() });
        assert!(!r.success);
        assert_eq!(r.message.as_deref(), Some("LaunchApp is disabled for voice."));
    }

    #[test]
    fn press_makes_a_shortcut_only_when_it_is_a_real_one() {
        assert_eq!(run("press control shift t").action, Some(shortcut("Ctrl+Shift+T")));
        assert_eq!(run("hotkey alt tab").action, Some(shortcut("Alt+Tab")));
        assert_eq!(run("shortcut control plus c").action, Some(shortcut("Ctrl+C")));
        assert!(run("press banana split").action.is_none() || run("press banana split").action != Some(shortcut("Banana+Split")));
    }

    #[test]
    fn switch_to_picks_the_nearest_program_or_profile() {
        let r = run("switch to chrome");
        assert_eq!(r.action, Some(action("LaunchApp", "Chrome")), "{r:?}");
        let none = run("switch to zzzzzz");
        assert!(!none.success);
        assert_eq!(none.message.as_deref(), Some("No app/profile match for \"zzzzzz\"."));
    }

    #[test]
    fn run_script_buttons_are_never_run_by_voice() {
        // "run backup" is an open-style command (launch an app named backup), never the script.
        assert_ne!(run("run backup").action.as_ref().map(|a| a.kind.as_str()), Some("RunScript"));
        let entry = lexicon().into_iter().find(|e| e.normalized == "run backup").unwrap();
        assert_eq!(resolve_entry(&entry, &RouterSettings::default()), Err("RunScript is blocked from voice.".into()));
        let automation = ButtonAction {
            kind: "Automation".into(),
            value: String::new(),
            steps: vec![crate::actions::ActionStep { kind: "RunScript".into(), value: "x".into(), ..Default::default() }],
        };
        assert!(is_run_script(&automation), "also hidden inside an automation");
    }

    #[test]
    fn noise_and_unknown_phrases_do_not_match() {
        assert_eq!(run("").message.as_deref(), Some("Empty transcript."));
        let r = run("the quick brown fox");
        assert!(!r.success && r.action.is_none(), "{r:?}");
    }

    #[test]
    fn two_equally_good_matches_are_not_guessed() {
        let a = LexiconEntry {
            phrase: "alpha one".into(),
            normalized: "alpha one".into(),
            kind: LexiconKind::ButtonLabel,
            action: Some(shortcut("A")),
            label: "ALPHA ONE".into(),
            profile_name: None,
            app_alias: None,
            prefer_active: false,
        };
        let b = LexiconEntry { phrase: "alpha two".into(), normalized: "alpha two".into(), action: Some(shortcut("B")), ..a.clone() };
        let r = route("alpha", &[a, b], &RouterSettings::default());
        assert!(!r.success, "ambiguous: {r:?}");
    }

    #[test]
    fn scoring_follows_the_csharp_rules() {
        assert_eq!(score("mute", "mute"), 1.0);
        assert!((score("open notepad now", "notepad") - 0.85 * 7.0 / 16.0).abs() < 1e-9);
        assert!((score("new tab page", "new tab thing") - (0.5 + 0.3 * 0.5)).abs() < 1e-9, "jaccard of 2 over 4");
        assert_eq!(score("", "x"), 0.0);
    }

    #[test]
    fn open_cloud_launches_claude_only_where_it_is_installed() {
        let on = |installed: &[&str]| RouterSettings { installed: Some(installed.iter().map(|s| s.to_string()).collect()), ..Default::default() };
        let launched = |s: &RouterSettings| route("open cloud", &[], s).action.map(|a| a.value);
        assert_eq!(launched(&on(&["claude"])).as_deref(), Some("claude"));
        assert_eq!(launched(&on(&[])).as_deref(), Some("cloud"), "left as the words said, which no app answers to");
    }

    #[test]
    fn close_plot_and_close_god_close_a_running_claude_only() {
        let s = running(&[("claude", "Claude")]);
        for said in ["close plot", "close god", "close claude"] {
            let r = route(said, &[], &s);
            assert_eq!(r.action.map(|a| a.value).as_deref(), Some("claude"), "{said}");
        }
        assert!(!route("close plot", &[], &running(&[("notepad", "Untitled - Notepad")])).success);
    }

    fn running(names: &[(&str, &str)]) -> RouterSettings {
        let running = names.iter().map(|(p, t)| RunningApp { process: (*p).into(), title: (*t).into() }).collect();
        RouterSettings { running, ..RouterSettings::default() }
    }

    #[test]
    fn close_names_a_program_that_is_open_now() {
        let open = running(&[("WhatsApp", "WhatsApp"), ("msedge", "Inbox - Outlook - Microsoft Edge"), ("Code", "main.rs - OASIS - Visual Studio Code")]);
        let closes = |said: &str| route(said, &[], &open).action.map(|a| format!("{}:{}", a.kind, a.value));
        assert_eq!(closes("close whatsapp").as_deref(), Some("CloseApp:WhatsApp"));
        assert_eq!(closes("close the whatsapp").as_deref(), Some("CloseApp:WhatsApp"));
        assert_eq!(closes("close edge").as_deref(), Some("CloseApp:msedge"), "a well-known other name");
        assert_eq!(closes("close vs code").as_deref(), Some("CloseApp:Code"));
        assert_eq!(closes("close browser").as_deref(), Some("CloseApp:msedge"), "the browser that is open");
        assert_eq!(closes("close whatsap").as_deref(), Some("CloseApp:WhatsApp"), "a close spelling");
    }

    #[test]
    fn close_with_nothing_open_by_that_name_does_nothing() {
        let open = running(&[("WhatsApp", "WhatsApp")]);
        let result = route("close spotify", &[], &open);
        assert!(!result.success && result.action.is_none(), "{result:?}");
        assert!(result.message.unwrap().contains("not open"));
        assert!(route("close the door", &[], &open).action.is_none());
        assert!(route("close parenthesis", &[], &open).action.is_none(), "spoken punctuation is not a program");
    }

    #[test]
    fn a_near_miss_is_offered_not_run() {
        let result = run("new tap");
        if result.success {
            return; // matched with confidence; nothing to suggest
        }
        if let Some((action, label)) = &result.suggestion {
            assert!(result.score >= SUGGEST_FROM, "{result:?}");
            assert!(!action.kind.is_empty() && !label.is_empty());
        }
        assert!(run("the weather is lovely today").suggestion.is_none(), "nothing close: no question");
    }

    /// Every editing phrase (`super::super::edit`) and the keys it presses, in order.
    const EDIT_PHRASES: &[(&str, &[&str])] = &[
        ("select all", &["Ctrl+A"]), ("select everything", &["Ctrl+A"]),
        ("copy", &["Ctrl+C"]), ("copy that", &["Ctrl+C"]),
        ("paste", &["Ctrl+V"]), ("paste it", &["Ctrl+V"]), ("paste that", &["Ctrl+V"]),
        ("cut", &["Ctrl+X"]), ("cut that", &["Ctrl+X"]),
        ("undo", &["Ctrl+Z"]), ("undo that", &["Ctrl+Z"]),
        ("redo", &["Ctrl+Y"]),
        ("paste as plain text", &["Ctrl+Shift+V"]),
        ("save", &["Ctrl+S"]), ("save this", &["Ctrl+S"]), ("save that", &["Ctrl+S"]), ("save it", &["Ctrl+S"]),
        ("find", &["Ctrl+F"]), ("bold", &["Ctrl+B"]), ("italic", &["Ctrl+I"]), ("underline", &["Ctrl+U"]),
        ("control a", &["Ctrl+A"]), ("ctrl a", &["Ctrl+A"]),
        ("control c", &["Ctrl+C"]), ("ctrl c", &["Ctrl+C"]), ("control see", &["Ctrl+C"]),
        ("control v", &["Ctrl+V"]), ("ctrl v", &["Ctrl+V"]),
        ("control x", &["Ctrl+X"]), ("ctrl x", &["Ctrl+X"]),
        ("control z", &["Ctrl+Z"]), ("ctrl z", &["Ctrl+Z"]), ("control zed", &["Ctrl+Z"]), ("ctrl zee", &["Ctrl+Z"]),
        ("control y", &["Ctrl+Y"]), ("ctrl y", &["Ctrl+Y"]),
        ("control s", &["Ctrl+S"]), ("ctrl s", &["Ctrl+S"]),
        ("control f", &["Ctrl+F"]), ("ctrl f", &["Ctrl+F"]),
        ("press enter", &["Enter"]), ("enter", &["Enter"]),
        ("press tab", &["Tab"]),
        ("press escape", &["Esc"]), ("escape", &["Esc"]),
        ("press backspace", &["Backspace"]),
        ("press space", &["Space"]),
        ("go to start of line", &["Home"]), ("go to the start of the line", &["Home"]),
        ("go to beginning of line", &["Home"]), ("go to the beginning of the line", &["Home"]),
        ("go to end of line", &["End"]), ("go to the end of the line", &["End"]),
        ("go to start of document", &["Ctrl+Home"]), ("go to the start of the document", &["Ctrl+Home"]),
        ("go to beginning of document", &["Ctrl+Home"]), ("go to the beginning of the document", &["Ctrl+Home"]),
        ("go to end of document", &["Ctrl+End"]), ("go to the end of the document", &["Ctrl+End"]),
        ("page up", &["PageUp"]), ("page down", &["PageDown"]),
        ("scroll up", &["PageUp"]), ("scroll down", &["PageDown"]),
        ("select line", &["Home", "Shift+End"]), ("select the line", &["Home", "Shift+End"]), ("select this line", &["Home", "Shift+End"]),
        ("select word", &["Ctrl+Right", "Ctrl+Shift+Left"]), ("select the word", &["Ctrl+Right", "Ctrl+Shift+Left"]), ("select this word", &["Ctrl+Right", "Ctrl+Shift+Left"]),
        ("select to end of line", &["Shift+End"]), ("select to the end of the line", &["Shift+End"]),
        ("select to start of line", &["Shift+Home"]), ("select to the start of the line", &["Shift+Home"]),
        ("select to beginning of line", &["Shift+Home"]), ("select to the beginning of the line", &["Shift+Home"]),
        ("refresh", &["F5"]), ("refresh page", &["F5"]), ("refresh the page", &["F5"]),
        ("reload", &["F5"]), ("reload page", &["F5"]), ("reload the page", &["F5"]),
        ("go back", &["Alt+Left"]), ("go forward", &["Alt+Right"]),
        ("next tab", &["Ctrl+Tab"]), ("previous tab", &["Ctrl+Shift+Tab"]),
        ("zoom in", &["Ctrl+="]), ("zoom out", &["Ctrl+-"]),
    ];

    fn keys_of(r: &RouteResult) -> Vec<String> {
        r.action.as_ref().map(|a| super::super::safety::steps_of(a).into_iter().filter(|(k, _)| k == "Shortcut").map(|(_, v)| v).collect()).unwrap_or_default()
    }

    #[test]
    fn editing_phrases_said_alone_press_their_keys() {
        for (said, keys) in EDIT_PHRASES {
            assert!(keys.iter().all(|k| Chord::is_valid(k)), "{keys:?} is a real shortcut");
            // As the speech model writes it: capitals, a full stop, "Ctrl-Z".
            let written = format!("{}.", said.replace("ctrl ", "Ctrl-"));
            for text in [said.to_string(), written] {
                let r = run(&text);
                assert!(r.success, "{text}: {r:?}");
                assert_eq!(keys_of(&r), *keys, "{text}");
                assert_eq!(super::super::safety::steps_of(r.action.as_ref().unwrap()).len(), keys.len(), "{text}: only key presses");
            }
        }
    }

    #[test]
    fn undo_cut_and_paste_run_unasked_and_delete_still_asks() {
        use super::super::safety::{fast_path, FastPath};
        let mode = super::super::ApprovalMode::FirstUse;
        for said in ["cut", "undo", "redo", "paste", "ctrl z", "select line", "go back", "zoom in", "press backspace"] {
            assert_eq!(fast_path(run(said).action.as_ref().unwrap(), &HashSet::new(), mode), FastPath::Run, "{said}");
        }
        assert_eq!(fast_path(run("press delete").action.as_ref().unwrap(), &HashSet::new(), mode), FastPath::Ask, "delete is not in the table and still asks");
    }

    #[test]
    fn editing_words_inside_a_sentence_are_not_editing_commands() {
        for said in [
            "copy the file", "I want to paste this later", "save the date please", "select all the things we discussed",
            "I need to copy the file", "undo the changes we made", "find my keys", "cut the cake", "go back home",
            "ctrl t", "control t", "control alt", "tab", "space", "backspace", "press the enter key now",
        ] {
            let r = run(said);
            let table_keys = EDIT_PHRASES.iter().map(|(_, k)| k.iter().map(|s| s.to_string()).collect::<Vec<_>>()).collect::<Vec<_>>();
            assert!(!r.success || !table_keys.contains(&keys_of(&r)), "{said}: {r:?}");
        }
        // The old "paste <text>" route keeps its text; only the whole phrase "paste it" is Ctrl+V.
        assert_eq!(run("paste hello world").action, Some(action("PasteText", "hello world")));
        assert_eq!(run("paste it").action, Some(shortcut("Ctrl+V")));
        assert_eq!(run("paste this later").action, Some(action("PasteText", "this later")));
    }

    #[test]
    fn with_editing_commands_off_the_old_routes_answer() {
        let off = RouterSettings { edit_commands: false, ..Default::default() };
        assert!(!route("copy", &lexicon(), &off).success);
        assert!(!route("ctrl z", &lexicon(), &off).success, "a bare chord alone is not a shortcut");
        assert_eq!(route("paste it", &lexicon(), &off).action, Some(action("PasteText", "it")));
        assert_eq!(route("press control c", &lexicon(), &off).action, Some(shortcut("Ctrl+C")), "the long form still works");
        assert_eq!(route("press enter", &lexicon(), &off).action, Some(shortcut("Enter")));
    }
}
