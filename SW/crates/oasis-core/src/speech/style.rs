//! How dictation is written for the program in front (`docs/VOICE_DECISIONS.md` V7,
//! `docs/VOICE_PIPELINE.md` §3.6). The style only shapes formatting: it never changes the words
//! said, and never decides whether a phrase is a command.

/// The writing style for one program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Chat: a single sentence gets no closing full stop.
    Message,
    /// Mail: full punctuation, paragraphs (today's rules).
    Mail,
    /// Notes and documents: paragraphs and lists (today's rules).
    Notes,
    /// Code editors and terminals: Dictionary and spoken punctuation only. No capitals, no full
    /// stop, no numbers rewritten, no corrections, no layout.
    Exact,
    /// Everything else: today's rules.
    Default,
}

// shortcut: a fixed table; make it editable in Settings when testers ask for an app that is missing.
const MESSAGE_APPS: [&str; 7] = ["whatsapp", "slack", "discord", "ms-teams", "teams", "telegram", "signal"];
const MAIL_APPS: [&str; 4] = ["outlook", "olk", "thunderbird", "mailspring"];
const NOTES_APPS: [&str; 5] = ["notion", "obsidian", "onenote", "winword", "evernote"];
// Terminals only: editors and IDEs also host chat and commit boxes, so they get the Default rules
// (spec 1.2). shortcut: no per-control split yet; map only the editor surface to Exact later.
const EXACT_APPS: [&str; 8] = ["windowsterminal", "wt", "cmd", "powershell", "pwsh", "conhost", "alacritty", "wezterm-gui"];
/// Web versions, by the start of the address's host.
const MESSAGE_SITES: [&str; 5] = ["web.whatsapp.com", "app.slack.com", "discord.com", "teams.microsoft.com", "web.telegram.org"];
const MAIL_SITES: [&str; 3] = ["mail.google.com", "outlook.live.com", "outlook.office.com"];
const NOTES_SITES: [&str; 3] = ["notion.so", "www.notion.so", "docs.google.com"];

/// The style for `app` (process name without ".exe") and, in a browser, the page's `url`.
pub fn for_app(app: &str, url: &str) -> Style {
    let app = app.trim().to_lowercase();
    let app = app.strip_suffix(".exe").unwrap_or(&app);
    let host = url.trim().split("://").last().unwrap_or("").split(['/', '?', '#']).next().unwrap_or("").to_lowercase();
    let on = |sites: &[&str]| !host.is_empty() && sites.contains(&host.as_str());
    if MESSAGE_APPS.contains(&app) || on(&MESSAGE_SITES) {
        Style::Message
    } else if MAIL_APPS.contains(&app) || on(&MAIL_SITES) {
        Style::Mail
    } else if NOTES_APPS.contains(&app) || on(&NOTES_SITES) {
        Style::Notes
    } else if EXACT_APPS.contains(&app) {
        Style::Exact
    } else {
        Style::Default
    }
}

/// A casual tone picked in Settings is for everyday writing, not only chat: an app with no style of
/// its own then writes like chat. Mail, notes and terminals keep theirs.
pub fn with_tone(style: Style, casual: bool) -> Style {
    if casual && style == Style::Default { Style::Message } else { style }
}

/// Chat style: one sentence loses its closing full stop ("Sounds good." → "Sounds good"). A
/// question or exclamation mark, an ellipsis, or more than one sentence stays as it is.
pub fn for_message(text: &str) -> String {
    let body = text.trim_end();
    let Some(stripped) = body.strip_suffix('.') else { return text.to_string() };
    let one_sentence = !stripped.contains(['.', '!', '?', '\n']);
    if one_sentence && !stripped.ends_with('.') { stripped.to_string() } else { text.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_casual_tone_writes_everyday_apps_like_chat() {
        assert_eq!(with_tone(Style::Default, true), Style::Message);
        assert_eq!(with_tone(Style::Default, false), Style::Default);
        for kept in [Style::Mail, Style::Notes, Style::Exact, Style::Message] {
            assert_eq!(with_tone(kept, true), kept);
        }
    }

    #[test]
    fn programs_and_sites_get_their_style() {
        assert_eq!(for_app("WhatsApp", ""), Style::Message);
        assert_eq!(for_app("OUTLOOK.EXE", ""), Style::Mail);
        assert_eq!(for_app("Code", ""), Style::Default);
        assert_eq!(for_app("cursor", ""), Style::Default);
        assert_eq!(for_app("cmd.exe", ""), Style::Exact);
        assert_eq!(for_app("WindowsTerminal", ""), Style::Exact);
        assert_eq!(for_app("chrome", "https://web.whatsapp.com/"), Style::Message);
        assert_eq!(for_app("msedge", "mail.google.com/mail/u/0/#inbox"), Style::Mail);
        assert_eq!(for_app("chrome", "https://docs.google.com/document/d/x"), Style::Notes);
        assert_eq!(for_app("chrome", "https://example.com/web.whatsapp.com"), Style::Default);
        assert_eq!(for_app("notepad", ""), Style::Default);
    }

    #[test]
    fn chat_drops_only_a_single_sentence_full_stop() {
        assert_eq!(for_message("Sounds good."), "Sounds good");
        assert_eq!(for_message("Are you in?"), "Are you in?");
        assert_eq!(for_message("Wait..."), "Wait...");
        assert_eq!(for_message("I'm late. Start without me."), "I'm late. Start without me.");
        assert_eq!(for_message("Meet at 3.30."), "Meet at 3.30.", "a number's point counts as a sentence mark; kept to be safe");
    }
}
