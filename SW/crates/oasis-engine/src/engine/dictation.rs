//! Dictation: cleanup, polish, the trace log, the talk cues, and typing the text where the cursor is.

use super::*;

/// How the program in front wants dictation written (`docs/VOICE_DECISIONS.md` V7).
pub(crate) fn dictation_style(foreground: &mut dyn ForegroundProbe) -> Style {
    foreground.current().map_or(Style::Default, |info| {
        let context = ForegroundContext::from(&info);
        oasis_core::speech::style::for_app(&context.app_name, &context.url)
    })
}

/// What was heard goes to the focused app as typing, like a keyboard. In preview mode, or with a
/// bad signature, it is shown and not typed.
/// `keep_on_clipboard`: no text field was found, so the paste may land nowhere; the text then stays on
/// the clipboard for the user to paste (the paste puts the old clipboard back, which is this text).
///
/// Cleanup and the S1-mini polish can take seconds, so they run on the action worker, in this job's
/// place in the queue: the engine loop keeps reading the pad, and a press or command that comes
/// meanwhile still runs after this text is pasted.
pub(super) fn type_dictation(ctx: &SpokenContext<'_>, heard: Heard, style: Style, keep_on_clipboard: bool) {
    let (shared, settings, verified) = (Arc::clone(ctx.shared), ctx.settings.clone(), ctx.verified);
    let allow_run_script = settings.allow_run_script_actions;
    let prepare = move || dictation_text(&shared, &settings, verified, &heard, style, keep_on_clipboard);
    let action = ButtonAction { kind: ActionKind::PasteText.as_str().into(), value: String::new(), steps: vec![] };
    let what = "type what was said".to_string();
    let _ = ctx.jobs.send(Job { action, button: -1, what, allow_run_script, prepare: Some(Box::new(prepare)) });
}

/// A dictation cleaned, polished, kept and shown; the text to paste, or `None` when it is only shown.
pub(super) fn dictation_text(shared: &Shared, settings: &Settings, verified: bool, heard: &Heard, style: Style, keep_on_clipboard: bool) -> Option<String> {
    let raw = heard.text.as_str();
    let mut trace = Vec::new();
    let text = clean_dictation(settings, raw, style, &heard.paragraphs, &mut trace);
    if let Some(note) = crate::polish::take_note() {
        SecurityLog::at(shared.paths.security_log()).write(&note);
    }
    if VoiceSettings::from(settings).keep_history && !text.trim().is_empty() {
        let now = local_now();
        let saved = crate::history::save(&shared.paths.dictation_history_folder(), now.unix, now.offset_secs, &format!("{style:?}"), raw, &text, heard.audio.as_ref());
        if let Err(why) = saved {
            SecurityLog::at(shared.paths.security_log()).write(&format!("The dictation history could not be saved: {why}"));
        }
    }
    let voice = VoiceSettings::from(settings);
    let typed = if voice.paste_tag { format!("{text} [typed using project OASIS] ") } else { format!("{text} ") };
    // Paste off: still shown and kept, never typed or put on the clipboard.
    let verified = verified && voice.paste_dictation;
    if keep_on_clipboard && verified && shared.live.load(Ordering::SeqCst) {
        let _ = oasis_win::clipboard::set_text(&typed);
        shared.emit(EngineEvent::Note { text: "No text box was in focus, so this may not have been typed. It is on the clipboard: press Ctrl+V in the box you want.".into() });
    }
    if VoiceSettings::from(settings).keep_trace {
        write_dictation_trace(shared, style, raw, &trace, &typed);
    }
    show_typed(shared, settings, verified, &text).then_some(typed)
}

/// One block per dictation in `dictation-trace.log`: the speech model's raw text, the text after
/// every cleanup stage, and exactly what is pasted. Line breaks are written as `\n` inside quotes.
pub(super) fn write_dictation_trace(shared: &Shared, style: Style, raw: &str, trace: &[(String, String)], typed: &str) {
    let mut block = format!("dictation, style {style:?}\n  {:<16} {raw:?}", "1 raw (STT)");
    let mut last = raw.trim().to_string();
    for (stage, text) in trace {
        let mark = if *text == last { "  (same)" } else { "" };
        block.push_str(&format!("\n  {stage:<16} {text:?}{mark}"));
        last = text.clone();
    }
    block.push_str(&format!("\n  {:<16} {typed:?}", "3 typed"));
    SecurityLog::at(shared.paths.dictation_trace_log()).write(&block);
}

/// The click when the mic opens, and the one when the text lands. Both are trimmed to their sound.
/// The start click has no lead-in (it used to wait 400 ms); `VOICE_WAKE`, the near-silent noise
/// that was that lead-in, plays at the press instead, to wake an idle Bluetooth speaker (make_cues.py).
pub(super) const VOICE_IN: &[u8] = include_bytes!("../../assets/voice-in.wav");
pub(super) const VOICE_OUT: &[u8] = include_bytes!("../../assets/voice-out.wav");
pub(super) const VOICE_WAKE: &[u8] = include_bytes!("../../assets/voice-wake.wav");

/// Plays one of the two cues, unless Settings › Pad › Sound is off.
pub(super) fn cue(settings: &Settings, wav: &'static [u8]) {
    let voice = VoiceSettings::from(settings);
    if !voice.sounds {
        return;
    }
    if voice.speaker.is_empty() {
        oasis_win::system::play_wav(wav);
    } else {
        oasis_voice::cue::play(wav, &voice.speaker, oasis_win::system::play_wav);
    }
}

/// Type `value` where the cursor is, and show `shown` in the activity list. In preview mode, or
/// with a bad signature, it is only shown.
pub(super) fn paste_typed(shared: &Shared, settings: &Settings, verified: bool, jobs: &Jobs, shown: &str, value: String) {
    if show_typed(shared, settings, verified, shown) {
        let action = ButtonAction { kind: ActionKind::PasteText.as_str().into(), value, steps: vec![] };
        let allow_run_script = settings.allow_run_script_actions;
        let what = "type what was said".to_string();
        let _ = jobs.send(Job { action, button: -1, what, allow_run_script, prepare: None });
    }
}

/// Shows `shown` in the activity list and, when it will be typed, plays the end cue. True when it
/// is to be typed (actions on, signature good); nothing is shown for an empty text.
pub(super) fn show_typed(shared: &Shared, settings: &Settings, verified: bool, shown: &str) -> bool {
    if shown.is_empty() {
        return false;
    }
    let runs = verified && shared.live.load(Ordering::SeqCst);
    shared.emit(EngineEvent::Transcript { text: shown.to_string(), typed: runs });
    if runs {
        cue(settings, VOICE_OUT);
    }
    runs
}

/// Dictation as it will be typed: spoken punctuation, numbers, corrections, fillers, the user's own
/// spellings, capitals and lists, unless the user turned cleanup off; then their replacements.
/// `paragraphs`: the opening words after each long pause. The whole text is cleaned at once, so a
/// correction or a list can reach across a pause; a break is then placed only where those words
/// still open a sentence (V7). "…by Friday. [pause] No, Saturday." has no sentence left to break at.
pub(super) fn clean_dictation(settings: &Settings, raw: &str, style: Style, paragraphs: &[String], trace: &mut Vec<(String, String)>) -> String {
    let voice = VoiceSettings::from(settings);
    let mut context = CleanupContext::dictate(true);
    context.style = style;
    context.glossary = voice.dictation_glossary.iter().map(|(s, c)| GlossaryEntry::new(s, c)).collect();
    let mut result = if voice.dictation_cleanup {
        cleanup_pipeline::run_traced(&CleanupRequest::new(raw, context.clone()), trace)
    } else {
        trace.push(("cleanup off".into(), raw.trim().to_string()));
        cleanup_pipeline::identity(&CleanupRequest::new(raw.trim(), context.clone()))
    };
    if style != Style::Exact {
        result.corrected_text = oasis_core::speech::layout::place_paragraphs(&result.corrected_text, paragraphs);
        trace.push(("2 rules + pauses".into(), result.corrected_text.clone()));
        // S1-mini, when this PC passed its test (V16); guarded and time-limited (V18).
        if voice.dictation_cleanup {
            let keep: Vec<String> = voice.dictation_glossary.iter().map(|(_, c)| c.clone()).collect();
            result = crate::polish::apply(result, &context, &keep);
            trace.push(("S1-mini polish".into(), result.corrected_text.clone()));
            for why in crate::polish::take_rejections() {
                trace.push(("polish rejected".into(), why));
            }
        }
    }
    let typed = oasis_core::speech::glossary::replace(&result.corrected_text, &voice.replacements);
    trace.push(("Replacements".into(), typed.clone()));
    typed
}
