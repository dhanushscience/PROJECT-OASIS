//! What a spoken command may do without asking. Ports of `IntentSafetyPolicy` (the steps part),
//! `IntentConfirmationPolicy`, `IntentApprovalMode`, `IntentTrustedTargetStore.Remember`, and
//! `VoiceFastPathGate`.
//!
//! The fast router may run an action by itself, but only one the full safety check would run
//! without asking. So "press alt f4" or "open cmd" never skips the policy. Anything else needs a
//! pad press: button 1 runs it, button 2 cancels.

use std::collections::HashSet;

use crate::actions::{ActionKind, ButtonAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyTier {
    Safe,
    /// Allowed, but only after the user approves it.
    Caution,
    Blocked,
}

/// How much of what is said waits for a pad press. The numbers are stored in `profiles.json`:
/// append only, never renumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApprovalMode {
    /// The first time each app, link or paste target is used, then trusted.
    #[default]
    FirstUse = 0,
    /// Every command with an action waits, however routine.
    Always = 1,
    /// Routine commands run by themselves; only risky ones ask.
    RiskyOnly = 2,
}

impl ApprovalMode {
    pub fn from_number(n: i64) -> ApprovalMode {
        match n {
            1 => ApprovalMode::Always,
            2 => ApprovalMode::RiskyOnly,
            _ => ApprovalMode::FirstUse,
        }
    }
}

/// Phrases that are never allowed anywhere in a step.
const NEVER: [&str; 24] = [
    "shutdown", "shut down", "restart", "reboot", "log off", "logoff", "sign out", "format c:",
    "format d:", "del /f", "del /s", "rm -rf", "rmdir /s", "reg delete", "reg add", "diskpart",
    "cipher /w", "bcdedit", "delete all", "delete my files", "wipe", "factory reset", "empty recycle",
    "ignore previous instructions",
];
const NEVER_MORE: [&str; 4] = ["empty the recycle", "clear recycle", "clear my recycle", "ignore all previous"];

const BLOCKED_SHORTCUTS: [&str; 3] = ["Alt+F4", "Ctrl+Alt+Del", "Ctrl+Shift+Esc"];
const SHELL_TOKENS: [&str; 10] = [
    "cmd", "powershell", "pwsh", "wscript", "cscript", "mshta", "wt", "wsl", "bash",
    "microsoft.windowsterminal",
];

/// Whether any word of a launch value (split on spaces, quotes, path slashes, and the `_` and `!`
/// of Store app ids) is a shell, with or without `.exe`: `powershell`, `cmd /c x`,
/// `"C:\Windows\System32\cmd.exe"`, `Microsoft.WindowsTerminal_8wekyb3d8bbwe!App`.
pub fn names_a_shell(value: &str) -> bool {
    value.to_lowercase().split(|c: char| c.is_whitespace() || matches!(c, '\\' | '/' | '"' | '\'' | '_' | '!'))
        .any(|word| SHELL_TOKENS.contains(&word.strip_suffix(".exe").unwrap_or(word)))
}

fn contains_ci(text: &str, needle: &str) -> bool {
    text.to_lowercase().contains(&needle.to_lowercase())
}

/// The tier of one step `(type, value)`.
pub fn step_tier(kind: &str, value: &str) -> SafetyTier {
    let (kind, value) = (kind.trim(), value.trim());
    let k = ActionKind::of_ignore_case(kind);
    if matches!(k, ActionKind::RunScript | ActionKind::AutoHotkey | ActionKind::MouseClick | ActionKind::DeleteFile | ActionKind::MoveFile) {
        return SafetyTier::Blocked;
    }
    if k == ActionKind::LaunchApp && names_a_shell(value) {
        return SafetyTier::Blocked;
    }
    if k == ActionKind::Shortcut {
        if BLOCKED_SHORTCUTS.iter().any(|s| contains_ci(value, s)) {
            return SafetyTier::Blocked;
        }
        // Owner rule (2026-11-13): every close or delete asks. Delete/Del in any chord, and the
        // close-tab/close-window chords Ctrl+W, Ctrl+F4, Ctrl+Shift+W.
        let keys: Vec<String> = value.to_lowercase().split(['+', ',', ' ']).map(str::to_string).collect();
        let has = |k: &str| keys.iter().any(|x| x == k);
        if has("del") || has("delete") || ((has("ctrl") || has("control")) && (has("w") || has("f4"))) {
            return SafetyTier::Caution;
        }
    }
    if k == ActionKind::PasteText && !value.is_empty() && ["del ", "rm ", "format "].iter().any(|d| contains_ci(value, d)) {
        return SafetyTier::Caution;
    }
    if k == ActionKind::Radio && matches!(value, "wifi|off" | "bluetooth|off" | "airplane|on") {
        return SafetyTier::Caution;
    }
    if k == ActionKind::LockPc {
        return SafetyTier::Caution;
    }
    // Every close asks first (owner rule 2026-11-13, replaces W8's "one window does not ask").
    let closes_a_window = k == ActionKind::WindowCommand && value.to_lowercase().starts_with("close|");
    if k == ActionKind::CloseWindow || k == ActionKind::CloseApp || closes_a_window {
        return SafetyTier::Caution;
    }
    let corpus = format!("{kind} {value}").to_lowercase();
    if ["empty recycle", "clear recycle", "delete recycle"].iter().any(|p| corpus.contains(p)) {
        return SafetyTier::Caution;
    }
    SafetyTier::Safe
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyEvaluation {
    pub blocked: bool,
    pub block_reason: Option<String>,
    pub requires_confirmation: bool,
}

/// Evaluate a plan made of these steps.
pub fn evaluate(steps: &[(String, String)]) -> SafetyEvaluation {
    let corpus = steps.iter().map(|(t, v)| format!("{t} {v}")).collect::<Vec<_>>().join(" ").to_lowercase();
    if let Some(pattern) = NEVER.iter().chain(NEVER_MORE.iter()).find(|p| corpus.contains(**p)) {
        return SafetyEvaluation {
            blocked: true,
            block_reason: Some(format!("Blocked dangerous command pattern: {pattern}")),
            requires_confirmation: false,
        };
    }
    let mut caution = false;
    for (kind, value) in steps {
        match step_tier(kind, value) {
            SafetyTier::Blocked => {
                return SafetyEvaluation {
                    blocked: true,
                    block_reason: Some(format!("Blocked step: {kind} / {value}")),
                    requires_confirmation: false,
                };
            }
            SafetyTier::Caution => caution = true,
            SafetyTier::Safe => {}
        }
    }
    SafetyEvaluation { blocked: false, block_reason: None, requires_confirmation: caution }
}

// ---- confirmation ------------------------------------------------------------------------------

/// Apps, links and pasted text are "first-use sensitive": they ask the first time.
pub fn is_first_use_sensitive(kind: &str) -> bool {
    matches!(ActionKind::of_ignore_case(kind.trim()), ActionKind::LaunchApp | ActionKind::OpenUrl | ActionKind::PasteText)
}

/// The key a target is remembered under: `launchapp:chrome`, `openurl:https://example.com`.
pub fn target_key(kind: &str, value: &str) -> String {
    let kind = kind.trim().to_lowercase();
    let target = normalize_target(&kind, value);
    format!("{kind}:{target}")
}

fn normalize_target(kind: &str, value: &str) -> String {
    let v = value.trim().trim_matches('"');
    match ActionKind::of_ignore_case(kind) {
        // A known search site is trusted per site, not per query, or every new voice search would wait
        // for a pad press. Read-only results pages only; any other address keys on the whole URL.
        ActionKind::OpenUrl => super::web_search::search_endpoint(v).unwrap_or_else(|| v.trim_end_matches('/').to_lowercase()),
        ActionKind::LaunchApp => match v.split_once('|') {
            Some((browser, url)) if !browser.trim().is_empty() => match super::web_search::search_endpoint(url) {
                Some(endpoint) => format!("{}|{endpoint}", normalize_launch_target(browser.trim())),
                None => normalize_launch_target(v),
            },
            _ => normalize_launch_target(v),
        },
        _ => v.chars().take(200).collect::<String>().to_lowercase(),
    }
}

/// A program named bare ("spotify") is trusted by that name, without `.exe`. One given by a full
/// path is trusted by the whole path: a `Spotify.exe` dropped in Downloads must not inherit the trust
/// given to the installed Spotify.
fn normalize_launch_target(value: &str) -> String {
    let bytes = value.as_bytes();
    let absolute = value.starts_with("\\\\") || (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/'));
    if absolute {
        return value.replace('/', "\\").to_lowercase();
    }
    let name = value.rsplit(['\\', '/']).next().filter(|n| !n.is_empty()).unwrap_or(value);
    let name = name.strip_suffix(".exe").or_else(|| name.strip_suffix(".EXE")).unwrap_or(name);
    name.to_lowercase()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmationDecision {
    pub required: bool,
    pub caution: bool,
    pub first_use: bool,
    pub every_time: bool,
}

/// Whether the steps must wait for a pad press. Caution steps always ask; the mode decides the
/// routine rest. (Cloud planners always asked in C#; there are none here.)
pub fn decide(
    caution: bool,
    steps: &[(String, String)],
    trusted: &HashSet<String>,
    mode: ApprovalMode,
) -> ConfirmationDecision {
    let first_use = mode == ApprovalMode::FirstUse
        && steps.iter().filter(|(t, v)| needs_trust(t, v)).any(|(t, v)| {
            let key = target_key(t, v);
            !trusted.iter().any(|known| known.eq_ignore_ascii_case(&key))
        });
    // A plan of nothing but pauses does nothing, so there is nothing to approve.
    let every_time = mode == ApprovalMode::Always && steps.iter().any(|(t, _)| ActionKind::of_ignore_case(t) != ActionKind::Delay);
    ConfirmationDecision { required: caution || first_use || every_time, caution, first_use, every_time }
}

/// The targets in these steps that the user may be asked to trust.
pub fn sensitive_target_keys(steps: &[(String, String)]) -> Vec<String> {
    steps.iter().filter(|(t, v)| needs_trust(t, v)).map(|(t, v)| target_key(t, v)).collect()
}

/// A first-use target, unless it only opens something to look at: a listed Windows place, a folder
/// or a plain document (`docs/OPEN_ANYTHING.md` O10). Programs, scripts and shortcuts still ask.
fn needs_trust(kind: &str, value: &str) -> bool {
    let opens_only = ActionKind::of_ignore_case(kind.trim()) == ActionKind::LaunchApp
        && (crate::actions::places::is_known(value.trim()) || super::on_screen::is_plain_open(value));
    is_first_use_sensitive(kind) && !opens_only
}

/// The steps of an action: an automation's own steps, or the single step.
pub fn steps_of(action: &ButtonAction) -> Vec<(String, String)> {
    if ActionKind::of_ignore_case(&action.kind) == ActionKind::Automation {
        action.steps.iter().map(|s| (s.kind.clone(), s.value.clone())).collect()
    } else {
        vec![(action.kind.clone(), action.value.clone())]
    }
}

/// What the fast path may do with an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastPath {
    /// Run it now.
    Run,
    /// Ask first: button 1 runs it, button 2 cancels.
    Ask,
    /// Never.
    Refuse,
}

/// The fast path may run an action only if the full policy would run it without asking.
pub fn fast_path(action: &ButtonAction, trusted: &HashSet<String>, mode: ApprovalMode) -> FastPath {
    let steps = steps_of(action);
    let safety = evaluate(&steps);
    if safety.blocked {
        return FastPath::Refuse;
    }
    let decision = decide(safety.requires_confirmation, &steps, trusted, mode);
    if decision.required { FastPath::Ask } else { FastPath::Run }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::ActionStep;

    fn act(kind: &str, value: &str) -> ButtonAction {
        ButtonAction { kind: kind.into(), value: value.into(), steps: vec![] }
    }

    fn trusted(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| (*k).to_string()).collect()
    }

    #[test]
    fn dangerous_shortcuts_and_shells_are_refused() {
        for (kind, value) in [("Shortcut", "Alt+F4"), ("Shortcut", "ctrl+alt+del"), ("LaunchApp", "C:\\Windows\\System32\\cmd.exe"), ("LaunchApp", "powershell.exe"), ("LaunchApp", "powershell"), ("LaunchApp", "PowerShell"), ("LaunchApp", "cmd"), ("LaunchApp", "cmd /c calc"), ("LaunchApp", r"C:\Program Files\PowerShell\7\pwsh"), ("LaunchApp", "\"mshta.exe\" x"), ("LaunchApp", "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"), ("LaunchApp", r"C:\Program Files\Git\bin\bash.exe"), ("LaunchApp", "wsl"), ("RunScript", "x"), ("MouseClick", "1,2")] {
            assert_eq!(fast_path(&act(kind, value), &trusted(&[]), ApprovalMode::FirstUse), FastPath::Refuse, "{kind} {value}");
        }
    }

    #[test]
    fn names_that_only_contain_a_shell_word_are_not_blocked() {
        for value in ["cmder", "notepad", "bashful", "chrome|https://example.com/cmd-guide", r"C:\Program Files\SEGGER\JLink\JLink.exe"] {
            assert!(!names_a_shell(value), "{value}");
        }
    }

    #[test]
    fn dangerous_words_anywhere_are_refused() {
        assert_eq!(fast_path(&act("PasteText", "please shutdown now"), &trusted(&[]), ApprovalMode::FirstUse), FastPath::Refuse);
        assert_eq!(fast_path(&act("PasteText", "rm -rf /"), &trusted(&[]), ApprovalMode::RiskyOnly), FastPath::Refuse);
    }

    #[test]
    fn risky_but_allowed_steps_always_ask_in_every_mode() {
        for mode in [ApprovalMode::FirstUse, ApprovalMode::Always, ApprovalMode::RiskyOnly] {
            assert_eq!(fast_path(&act("Shortcut", "Ctrl+W"), &trusted(&[]), mode), FastPath::Ask, "{mode:?}");
            assert_eq!(fast_path(&act("PasteText", "del everything"), &trusted(&[]), mode), FastPath::Ask, "{mode:?}");
        }
    }

    #[test]
    fn every_close_and_delete_asks_in_every_mode() {
        for mode in [ApprovalMode::FirstUse, ApprovalMode::Always, ApprovalMode::RiskyOnly] {
            for (kind, value) in [("WindowCommand", "close|"), ("WindowCommand", "close|chrome"), ("CloseApp", "WhatsApp"), ("Shortcut", "Delete"), ("Shortcut", "Del"), ("Shortcut", "Shift+Del"), ("Shortcut", "Ctrl+Delete"), ("Shortcut", "Ctrl+F4"), ("Shortcut", "Ctrl+Shift+W"), ("Shortcut", "ctrl+w")] {
                assert_eq!(fast_path(&act(kind, value), &trusted(&[]), mode), FastPath::Ask, "{kind} {value} {mode:?}");
            }
        }
        assert_eq!(fast_path(&act("WindowCommand", "maximize|chrome"), &trusted(&[]), ApprovalMode::RiskyOnly), FastPath::Run);
        assert_eq!(fast_path(&act("Shortcut", "Ctrl+Win"), &trusted(&[]), ApprovalMode::RiskyOnly), FastPath::Run, "Win is not W");
    }

    #[test]
    fn media_keys_run_without_asking() {
        assert_eq!(fast_path(&act("Shortcut", "VolumeUp"), &trusted(&[]), ApprovalMode::FirstUse), FastPath::Run);
        assert_eq!(fast_path(&act("Shortcut", "Ctrl+T"), &trusted(&[]), ApprovalMode::Always), FastPath::Ask, "Always asks about everything");
    }

    #[test]
    fn trusting_one_search_trusts_every_query_on_that_site() {
        let t = trusted(&["openurl:https://www.google.com/search?q=", "launchapp:chrome|https://www.google.com/search?q="]);
        let open = act("OpenUrl", "https://www.google.com/search?q=laptops");
        let launch = act("LaunchApp", "chrome|https://www.google.com/search?q=laptops");
        assert_eq!(fast_path(&open, &t, ApprovalMode::FirstUse), FastPath::Run);
        assert_eq!(fast_path(&launch, &t, ApprovalMode::FirstUse), FastPath::Run);
        let other = act("OpenUrl", "https://www.amazon.in/s?k=laptops");
        assert_eq!(fast_path(&other, &t, ApprovalMode::FirstUse), FastPath::Ask);
        let brave = act("LaunchApp", "brave|https://www.google.com/search?q=laptops");
        assert_eq!(fast_path(&brave, &t, ApprovalMode::FirstUse), FastPath::Ask);
    }

    #[test]
    fn a_new_app_asks_once_and_a_trusted_one_runs() {
        let launch = act("LaunchApp", "Spotify");
        assert_eq!(fast_path(&launch, &trusted(&[]), ApprovalMode::FirstUse), FastPath::Ask);
        assert_eq!(fast_path(&launch, &trusted(&["launchapp:spotify"]), ApprovalMode::FirstUse), FastPath::Run);
        assert_eq!(fast_path(&launch, &trusted(&["LaunchApp:SPOTIFY"]), ApprovalMode::FirstUse), FastPath::Run, "case does not matter");
        // A program by full path is trusted by that path only: a planted copy does not borrow the name's trust.
        let planted = act("LaunchApp", r"C:\Users\me\Downloads\Spotify.exe");
        assert_eq!(fast_path(&planted, &trusted(&["launchapp:spotify"]), ApprovalMode::FirstUse), FastPath::Ask);
        assert_eq!(fast_path(&planted, &trusted(&[r"launchapp:c:\users\me\downloads\spotify.exe"]), ApprovalMode::FirstUse), FastPath::Run);
        assert_eq!(fast_path(&launch, &trusted(&[]), ApprovalMode::RiskyOnly), FastPath::Run, "risky-only trusts routine apps");
        // Only looking: a place, a folder, a document. A program on the Desktop still asks.
        for value in ["shell:RecycleBinFolder", r"C:\Users\me\Desktop\Project\", r"C:\Users\me\Desktop\Report.pdf"] {
            assert_eq!(fast_path(&act("LaunchApp", value), &trusted(&[]), ApprovalMode::FirstUse), FastPath::Run, "{value}");
        }
        assert_eq!(fast_path(&act("LaunchApp", r"C:\Users\me\Desktop\setup.exe"), &trusted(&[]), ApprovalMode::FirstUse), FastPath::Ask);
        assert_eq!(fast_path(&act("LaunchApp", r"C:\Users\me\Desktop\Project\"), &trusted(&[]), ApprovalMode::Always), FastPath::Ask, "Always still asks");
    }

    #[test]
    fn target_keys_are_stable() {
        assert_eq!(target_key("LaunchApp", "C:\\Apps\\Notepad.exe"), r"launchapp:c:\apps\notepad.exe");
        assert_eq!(target_key("LaunchApp", "C:/Apps/Notepad.exe"), r"launchapp:c:\apps\notepad.exe");
        assert_eq!(target_key("LaunchApp", "Notepad.exe"), "launchapp:notepad");
        assert_eq!(target_key("LaunchApp", "\"Spotify\""), "launchapp:spotify");
        assert_eq!(target_key("OpenUrl", "https://Example.com/"), "openurl:https://example.com");
        assert_eq!(target_key("PasteText", "Hello"), "pastetext:hello");
        assert_eq!(target_key("PasteText", &"x".repeat(300)).len(), "pastetext:".len() + 200);
    }

    #[test]
    fn an_automation_is_judged_by_its_steps() {
        let automation = ButtonAction {
            kind: "Automation".into(),
            value: String::new(),
            steps: vec![
                ActionStep { kind: "Delay".into(), value: "500".into(), ..Default::default() },
                ActionStep { kind: "Shortcut".into(), value: "Alt+F4".into(), ..Default::default() },
            ],
        };
        assert_eq!(fast_path(&automation, &trusted(&[]), ApprovalMode::FirstUse), FastPath::Refuse);
        let pauses = ButtonAction { steps: vec![ActionStep { kind: "Delay".into(), value: "1".into(), ..Default::default() }], ..automation };
        assert_eq!(fast_path(&pauses, &trusted(&[]), ApprovalMode::Always), FastPath::Run, "only pauses: nothing to approve");
    }

    #[test]
    fn the_approval_mode_numbers_never_change() {
        assert_eq!(ApprovalMode::from_number(0), ApprovalMode::FirstUse);
        assert_eq!(ApprovalMode::from_number(1), ApprovalMode::Always);
        assert_eq!(ApprovalMode::from_number(2), ApprovalMode::RiskyOnly);
        assert_eq!(ApprovalMode::from_number(99), ApprovalMode::FirstUse);
    }

    #[test]
    fn sensitive_keys_cover_apps_links_and_pastes_only() {
        let steps = vec![
            ("LaunchApp".to_string(), "chrome".to_string()),
            ("Shortcut".to_string(), "Ctrl+T".to_string()),
            ("OpenUrl".to_string(), "https://a.test".to_string()),
        ];
        assert_eq!(sensitive_target_keys(&steps), ["launchapp:chrome", "openurl:https://a.test"]);
    }
}
