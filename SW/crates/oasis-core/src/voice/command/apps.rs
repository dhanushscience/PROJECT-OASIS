//! How people (and speech-to-text) say an app's name, and the browsers. Ports of
//! `Oasis.Core/Intent/SpokenAppNames.cs`, the window aliases of `IntentPlanNormalizer.cs`,
//! `KnownAppResolver.GuessAppFromText`, and `Services/Intent/BrowserOpenIntentParser.cs`.

use crate::actions::{try_allow, UrlScope};
use crate::matching::web_catalog::{compact, guess_url, try_resolve};
use crate::matching::YouTubeHosts;

use super::text::normalize;

/// The canonical launcher name and every way it is said (compared with spaces and punctuation
/// removed, so "vs code" and "vscode" are one).
const VARIANTS: [(&str, &[&str]); 14] = [
    ("code", &[
        "code", "vscode", "vscodes", "vscoded", "vscodeeditor", "vscodeapp", "vscodeide", "vscodium",
        "visualstudiocode", "visualstudiocodeeditor", "microsoftvisualstudiocode", "visualstudiocodes",
        "visualcode", "vsc", "vcode", "vscoat", "vscord", "vscoad", "vscold", "vscods", "vscood",
        "vscoed", "vscot", "vsco", "bscode", "bescode", "veescode", "viescode", "vecode", "vscodee",
        "codeeditor", "codeide", "codeapp", "visualstudiocoat", "visualstudiocord", "vscodeinsiders",
        "visualstudiocodeinsiders", "visualstudiocoding", "vscoding", "viascore", "viascode", "vscore",
    ]),
    ("cursor", &["cursor", "cursorai", "cursoride", "cursoreditor", "curser", "cursorapp", "cursa"]),
    ("chrome", &["chrome", "googlechrome", "chromebrowser", "crome", "krome"]),
    ("msedge", &["edge", "msedge", "microsoftedge", "edgebrowser"]),
    ("firefox", &["firefox", "mozillafirefox", "firefoxbrowser"]),
    ("brave", &["brave", "bravebrowser"]),
    ("notepad", &["notepad", "notepadapp", "notespad"]),
    ("calc", &["calc", "calculator", "calculatorapp"]),
    ("explorer", &["explorer", "fileexplorer", "myfiles", "windowsexplorer"]),
    ("spotify", &["spotify", "spotifyapp", "spotifymusic"]),
    ("kicad", &["kicad", "keycad", "kycad", "kikad", "kicadeditor"]),
    ("claude", &["claude", "claudecode"]),
    ("whatsapp", &["whatsapp", "whatsappapp"]),
    ("wispr flow", &["wisprflow", "whisperflow", "whisprflow", "wisperflow", "wisprflo", "whisperflowapp"]),
];

/// Mishearings that are also ordinary words ("cloud", "what's up"): they name the app only on a PC
/// where that app is installed, so the words are otherwise left for typing.
const MISHEARD: [(&str, &[&str]); 3] = [
    ("kicad", &["kaicad", "kaycad"]),
    ("claude", &["cloud", "clawed", "clod", "cloudcode"]),
    ("whatsapp", &["whatsup"]),
];

/// The name to look an app up by in the Start Menu, per canonical name.
const LAUNCH_NAMES: [(&str, &str); 14] = [
    ("code", "Visual Studio Code"),
    ("cursor", "Cursor"),
    ("chrome", "Google Chrome"),
    ("msedge", "Microsoft Edge"),
    ("firefox", "Firefox"),
    ("brave", "Brave"),
    ("notepad", "Notepad"),
    ("calc", "Calculator"),
    ("explorer", "File Explorer"),
    ("spotify", "Spotify"),
    ("kicad", "KiCad"),
    ("claude", "Claude"),
    ("whatsapp", "WhatsApp"),
    ("wispr flow", "Wispr Flow"),
];

/// Names `KnownAppResolver` has paths for, longest first when searching a phrase.
const KNOWN_ALIASES: [&str; 14] = [
    "cursor", "vscode", "code", "visualstudio", "devenv", "notepad", "calc", "calculator", "explorer",
    "chrome", "edge", "firefox", "idea", "pycharm",
];

/// The Start Menu names of the apps OASIS knows by heart ("Visual Studio Code", "Google Chrome").
pub fn known_launch_names() -> impl Iterator<Item = &'static str> {
    LAUNCH_NAMES.iter().map(|(_, name)| *name)
}

/// The launcher name for a spoken app name ("vs code" → `code`).
pub fn canonical_app(spoken: &str) -> Option<&'static str> {
    canonical_app_if(spoken, &|_| true)
}

/// [`canonical_app`], where a mishearing that is also a word counts only if `installed(canonical)`.
pub fn canonical_app_if(spoken: &str, installed: &dyn Fn(&str) -> bool) -> Option<&'static str> {
    let key = compact(spoken);
    if key.is_empty() {
        return None;
    }
    let find = |table: &[(&'static str, &[&str])]| table.iter().find(|(_, names)| names.iter().any(|n| compact(n) == key)).map(|(c, _)| *c);
    find(&VARIANTS).or_else(|| find(&MISHEARD).filter(|c| installed(c)))
}

/// The Start Menu name for a spoken app ("vs code" → "Visual Studio Code").
pub fn launch_name(spoken: &str) -> Option<&'static str> {
    let canonical = canonical_app(spoken)?;
    LAUNCH_NAMES.iter().find(|(c, _)| *c == canonical).map(|(_, name)| *name)
}

/// The first app named anywhere in a phrase, trying runs of up to four words, longest first
/// ("visual studio code" before "code").
pub fn find_app_in(phrase: &str) -> Option<&'static str> {
    find_app_in_if(phrase, &|_| true)
}

pub fn find_app_in_if(phrase: &str, installed: &dyn Fn(&str) -> bool) -> Option<&'static str> {
    let words: Vec<&str> = phrase.split([' ', '\t', ',', '.', '!', '?']).filter(|w| !w.is_empty()).collect();
    for length in (1..=words.len().min(4)).rev() {
        for start in 0..=words.len() - length {
            if let Some(found) = canonical_app_if(&words[start..start + length].join(" "), installed) {
                return Some(found);
            }
        }
    }
    None
}

/// `KnownAppResolver.GuessAppFromText`: a known spoken app, else a known alias found inside the
/// text, else "cursor" for a bare "ide".
pub fn guess_app_from_text(text: &str) -> Option<String> {
    guess_app_from_text_if(text, &|_| true)
}

/// [`guess_app_from_text`] with the install check of [`canonical_app_if`].
pub fn guess_app_from_text_if(text: &str, installed: &dyn Fn(&str) -> bool) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if let Some(spoken) = find_app_in_if(text, installed) {
        return Some(spoken.to_string());
    }
    let mut aliases = KNOWN_ALIASES;
    aliases.sort_by_key(|a| std::cmp::Reverse(a.len()));
    aliases.iter().find(|a| words.contains(a)).map(|a| (*a).to_string())
}

const WINDOW_ALIASES: [(&str, &str); 20] = [
    ("my files", "explorer"),
    ("files", "explorer"),
    ("file explorer", "explorer"),
    ("explorer", "explorer"),
    ("this pc", "explorer"),
    ("brave browser", "brave"),
    ("google chrome", "chrome"),
    ("chrome browser", "chrome"),
    ("microsoft edge", "msedge"),
    ("edge browser", "msedge"),
    ("visual studio code", "code"),
    ("vscode", "code"),
    ("whatsapp application", "WhatsApp"),
    ("whatsapp", "WhatsApp"),
    ("spotify application", "spotify"),
    ("cursor application", "cursor"),
    ("kick-head application", "kicad"),
    ("key cad", "kicad"),
    ("keycad", "kicad"),
    ("kicad", "kicad"),
];

/// A spoken window or app token → the name windows are matched by. Spaces and hyphens do not
/// matter.
pub fn resolve_window_alias(token: &str) -> Option<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    if let Some((_, alias)) = WINDOW_ALIASES.iter().find(|(k, _)| k.eq_ignore_ascii_case(trimmed)) {
        return Some((*alias).to_string());
    }
    let squeezed: String = lower.chars().filter(|c| !matches!(c, ' ' | '-' | '_')).collect();
    if let Some((_, alias)) = WINDOW_ALIASES.iter().find(|(k, _)| {
        let key: String = k.chars().filter(|c| !matches!(c, ' ' | '-' | '_')).collect();
        key.eq_ignore_ascii_case(&squeezed)
    }) {
        return Some((*alias).to_string());
    }
    // "calc" is not the Calculator window's process name, so it is not a window alias.
    canonical_app(trimmed).filter(|a| *a != "calc").map(str::to_string)
}

// ---- "open YouTube in Brave" -------------------------------------------------------------------

const OPEN_PREFIXES: [&str; 4] = ["open ", "launch ", "start ", "run "];
/// Words that join a site to its browser: "… in / on / with / using Chrome".
const BROWSER_JOINERS: [&str; 5] = [" in ", " on ", " with ", " using ", " of "];
const BROWSERS: [&str; 6] = ["brave", "chrome", "firefox", "msedge", "opera", "vivaldi"];

const BROWSER_ACTIONS: [(&str, &str); 5] = [
    ("new tab", "Ctrl+T"),
    ("new window", "Ctrl+N"),
    ("incognito window", "Ctrl+Shift+N"),
    ("new incognito window", "Ctrl+Shift+N"),
    ("close tab", "Ctrl+W"),
];

/// "open new tab in chrome" → `(Some("chrome"), "Ctrl+T")`; no browser named → `None` browser (the
/// focused window). Checked before site guessing, which would turn "new tab" into newtab.com.
pub fn parse_browser_action(spoken: &str) -> Option<(Option<String>, &'static str)> {
    let text = normalize(spoken);
    let mut rest = OPEN_PREFIXES.iter().find_map(|p| text.strip_prefix(*p)).unwrap_or(&text).trim();
    rest = ["a ", "an ", "the "].iter().find_map(|a| rest.strip_prefix(*a)).unwrap_or(rest);
    // "new vs code window": a new window of any known app, only when it is asked for.
    if let Some(app) = rest.strip_prefix("new ").and_then(|r| r.trim_end_matches(" please").strip_suffix(" window")) {
        if let Some(app) = canonical_app(app.trim()).filter(|a| *a != "calc" && *a != "spotify") {
            return Some((Some(app.to_string()), new_window_chord(app)));
        }
    }
    let (phrase, chord) = BROWSER_ACTIONS.iter().find(|(p, _)| rest.starts_with(*p))?;
    let tail = rest[phrase.len()..].trim();
    if tail.is_empty() {
        return Some((None, chord));
    }
    let named = strip_determiner(&trim_trailing_noise(BROWSER_JOINERS.iter().find_map(|j| tail.strip_prefix(j.trim_start()))?));
    if let Some(browser) = resolve_known_browser(&named) {
        return Some((Some(browser), chord));
    }
    // "open a new window in VS Code": not a browser, so only "new window" applies.
    if *phrase == "new window" {
        let app = canonical_app(&named).filter(|a| *a != "calc" && *a != "spotify")?;
        return Some((Some(app.to_string()), new_window_chord(app)));
    }
    None
}

/// VS Code and Cursor open a new file on Ctrl+N; their new window is Ctrl+Shift+N.
fn new_window_chord(app: &str) -> &'static str {
    if matches!(app, "code" | "cursor") { "Ctrl+Shift+N" } else { "Ctrl+N" }
}

/// "open youtube in brave" → `("brave", "https://www.youtube.com")`.
pub fn parse_browser_open(spoken: &str) -> Option<(String, String)> {
    let text = normalize(spoken);
    if parse_browser_action(&text).is_some() {
        return None;
    }
    let prefix = OPEN_PREFIXES.iter().find(|p| text.starts_with(**p))?;
    let remainder = text[prefix.len()..].trim();
    for joiner in BROWSER_JOINERS {
        let Some(at) = remainder.rfind(joiner).filter(|at| *at > 0) else { continue };
        let site = remainder[..at].trim();
        let browser_raw = trim_trailing_noise(remainder[at + joiner.len()..].trim());
        if site.is_empty() || browser_raw.is_empty() {
            continue;
        }
        // "in my google chrome": the possessive is not part of the browser's name.
        let Some(browser) = resolve_known_browser(&strip_determiner(&browser_raw)) else { continue };
        return resolve_site_url(site).map(|url| (browser, url));
    }
    None
}

fn strip_determiner(raw: &str) -> String {
    let value = raw.trim();
    for word in ["my ", "the ", "your "] {
        if value.len() > word.len() && value[..word.len()].eq_ignore_ascii_case(word) {
            return value[word.len()..].trim().to_string();
        }
    }
    value.to_string()
}

fn trim_trailing_noise(raw: &str) -> String {
    let mut value = raw.trim().to_string();
    for noise in [" please", " for me", " now", " right now"] {
        if value.to_lowercase().ends_with(noise) {
            value.truncate(value.len() - noise.len());
            value = value.trim().to_string();
        }
    }
    value
}

/// A real browser, as opposed to any window alias ("vscode" is an alias too).
fn resolve_known_browser(raw: &str) -> Option<String> {
    let mut token = raw.trim().trim_matches(['"', '\'']).to_string();
    for trim in [" please", " for me", " now", " browser", " application", " app"] {
        if token.to_lowercase().ends_with(trim) {
            token.truncate(token.len() - trim.len());
            token = token.trim().to_string();
        }
    }
    let browser = match resolve_window_alias(&token) {
        Some(alias) => alias,
        None => match token.to_lowercase().as_str() {
            "edge" => "msedge".to_string(),
            "google chrome" => "chrome".to_string(),
            other => other.to_string(),
        },
    };
    BROWSERS.contains(&browser.as_str()).then_some(browser)
}

fn resolve_site_url(site: &str) -> Option<String> {
    let mut token = site.trim().trim_matches(['"', '\'']).to_string();
    for trim in [" please", " for me", " now", " website", " site"] {
        if token.to_lowercase().ends_with(trim) {
            token.truncate(token.len() - trim.len());
            token = token.trim().to_string();
        }
    }
    if YouTubeHosts::is_spoken_site(Some(&token)) {
        return Some(YouTubeHosts::HOME.to_string());
    }
    if YouTubeHosts::is_spoken_music(Some(&token)) {
        return Some(YouTubeHosts::MUSIC.to_string());
    }
    let lower = token.to_lowercase();
    if token.contains('.') || lower.starts_with("http://") || lower.starts_with("https://") {
        return try_allow(&token, UrlScope::Automation).filter(|u| !u.is_empty());
    }
    let url = try_resolve(&token).map_or_else(|| guess_url(&token), str::to_string);
    try_allow(&url, UrlScope::Automation).filter(|u| !u.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn many_ways_of_saying_an_app_name_reach_one_launcher_name() {
        for said in ["vs code", "v s code", "VSCode", "visual studio code", "vs coat", "Visual Studio Code Insiders"] {
            assert_eq!(canonical_app(said), Some("code"), "{said}");
        }
        assert_eq!(canonical_app("google chrome"), Some("chrome"));
        assert_eq!(canonical_app("key cad"), Some("kicad"));
        assert_eq!(canonical_app("nothing like it"), None);
        assert_eq!(launch_name("vs code"), Some("Visual Studio Code"));
        assert_eq!(launch_name("spotify"), Some("Spotify"));
    }

    #[test]
    fn claude_whatsapp_and_kicad_mishearings_reach_their_apps() {
        for said in ["cloud", "clawed", "clod", "claude code", "cloud code", "Claude"] {
            assert_eq!(canonical_app(said), Some("claude"), "{said}");
        }
        assert_eq!(launch_name("clawed"), Some("Claude"));
        for said in ["what's up", "whats app", "whatsapp"] {
            assert_eq!(canonical_app(said), Some("whatsapp"), "{said}");
        }
        assert_eq!(launch_name("whats app"), Some("WhatsApp"));
        assert_eq!(canonical_app("kaicad"), Some("kicad"));
        assert_eq!(canonical_app("kay cad"), Some("kicad"));
        assert_eq!(canonical_app("water"), None);
    }

    #[test]
    fn word_like_mishearings_name_an_app_only_where_it_is_installed() {
        let has = |names: &'static [&'static str]| move |app: &str| names.contains(&app);
        let (claude, nothing) = (has(&["claude", "whatsapp", "kicad"]), has(&[]));
        for said in ["cloud", "clawed", "clod", "cloud code", "what's up", "kaicad", "kaycad"] {
            assert!(canonical_app_if(said, &claude).is_some(), "{said} installed");
            assert_eq!(canonical_app_if(said, &nothing), None, "{said} not installed");
        }
        assert_eq!(guess_app_from_text_if("cloud", &nothing), None, "an ordinary word");
        assert_eq!(guess_app_from_text_if("cloud code", &claude).as_deref(), Some("claude"));
        assert_eq!(canonical_app_if("claude", &nothing), Some("claude"), "the real name needs no check");
    }

    #[test]
    fn the_longest_run_of_words_wins_when_finding_an_app_in_a_phrase() {
        assert_eq!(find_app_in("please open visual studio code now"), Some("code"));
        assert_eq!(find_app_in("open spotify"), Some("spotify"));
        assert_eq!(find_app_in("open the garden"), None);
    }

    #[test]
    fn guessing_falls_back_to_known_aliases() {
        assert_eq!(guess_app_from_text("vs code").as_deref(), Some("code"));
        assert_eq!(guess_app_from_text("my favourite idea board").as_deref(), Some("idea"), "an alias inside the text");
        assert_eq!(guess_app_from_text("   "), None);
        assert_eq!(guess_app_from_text("zzz"), None);
    }

    #[test]
    fn aliases_match_whole_words_only() {
        assert_eq!(guess_app_from_text("the guide"), None, "no 'ide' fallback inside a word");
        assert_eq!(guess_app_from_text("the ide please"), None, "the bare-ide fallback is gone");
        assert_eq!(guess_app_from_text("cloud code").as_deref(), Some("claude"), "'cloud code' is Claude Code, not VS Code");
        assert_eq!(guess_app_from_text("decoded notes"), None);
        for (said, app) in [("vs code", "code"), ("code", "code"), ("cursor", "cursor"), ("chrome", "chrome")] {
            assert_eq!(guess_app_from_text(said).as_deref(), Some(app), "{said}");
        }
    }

    #[test]
    fn window_aliases_ignore_spaces_and_hyphens_and_calc_is_not_one() {
        assert_eq!(resolve_window_alias("Google Chrome").as_deref(), Some("chrome"));
        assert_eq!(resolve_window_alias("key-cad").as_deref(), Some("kicad"));
        assert_eq!(resolve_window_alias("file explorer").as_deref(), Some("explorer"));
        assert_eq!(resolve_window_alias("calc"), None);
        assert_eq!(resolve_window_alias("whatsapp").as_deref(), Some("WhatsApp"));
    }

    #[test]
    fn browser_actions_are_not_sites() {
        let p = |s: &str| parse_browser_action(s);
        assert_eq!(p("open new tab in chrome"), Some((Some("chrome".into()), "Ctrl+T")));
        assert_eq!(p("open a new window on edge please"), Some((Some("msedge".into()), "Ctrl+N")));
        assert_eq!(p("launch incognito window in my google chrome"), Some((Some("chrome".into()), "Ctrl+Shift+N")));
        assert_eq!(p("close tab"), Some((None, "Ctrl+W")));
        assert_eq!(p("open youtube in brave"), None);
        assert_eq!(p("open new tab in notepad"), None, "not a browser");
        assert_eq!(p("open a new window in vs code"), Some((Some("code".into()), "Ctrl+Shift+N")));
        assert_eq!(p("open new vs code window"), Some((Some("code".into()), "Ctrl+Shift+N")));
        assert_eq!(p("open a new window of file explorer"), Some((Some("explorer".into()), "Ctrl+N")));
        assert_eq!(p("open vs code"), None, "plain open reuses the last window");
        assert_eq!(parse_browser_open("open new tab in chrome"), None, "never newtab.com");
    }

    #[test]
    fn opening_a_site_in_a_browser_is_understood() {
        let parsed = |s: &str| parse_browser_open(s);
        assert_eq!(parsed("open youtube in brave"), Some(("brave".into(), "https://www.youtube.com".into())));
        assert_eq!(parsed("launch youtube music on chrome please"), Some(("chrome".into(), "https://music.youtube.com".into())));
        assert_eq!(parsed("open github with my google chrome"), Some(("chrome".into(), "https://github.com".into())));
        assert_eq!(parsed("open example.org using edge"), Some(("msedge".into(), "https://exampleorg.com".into())), "the dot is dropped by cleaning, as in C#");
        assert_eq!(parsed("open stack overflow in firefox"), Some(("firefox".into(), "https://stackoverflow.com".into())));
        assert_eq!(parsed("open notepad in vscode"), None, "vscode is a window alias, not a browser");
        assert_eq!(parsed("open youtube"), None, "no browser named");
        assert_eq!(parsed("play youtube in brave"), None);
    }
}
