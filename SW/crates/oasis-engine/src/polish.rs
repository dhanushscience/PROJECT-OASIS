//! Polish by S1-mini on the graphics card (`docs/VOICE_DECISIONS.md` V16, V17, V18).
//!
//! A llama.cpp server runs while the setting `Voice.PolishModel` is set (the model manager sets it
//! once the QUALITY set passed its test). Dictation is polished one sentence at a time:
//! - while the button is held, each finished sentence is polished in the background and kept in a
//!   cache ([`prewarm`]), so after release only the last sentence waits;
//! - after release, [`apply`] reads the cache, asks for what is missing, and gives up at
//!   the wait limit (600 ms or more, `Voice.PolishWaitMs`): whatever was not polished stays as the rules wrote it.
//! The whole result still goes through `meaning::accepts` (in `maybe_polish`), and a sentence that
//! loses a Dictionary term is thrown away. On any failure the rule-cleaned text is typed.

use std::collections::HashMap;
use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oasis_core::speech::pipeline as cleanup_pipeline;
use oasis_core::speech::polish::maybe_polish;
use oasis_core::speech::{meaning, CleanupContext, CleanupRequest, CleanupResult};
use oasis_settings::Settings;

/// Least time polish may add after the button is released (the Voice screen can raise it).
const MIN_WAIT_MS: u64 = 600;
const MAX_WAIT_MS: u64 = 5_000;
/// The five tones S1-mini answers to, from casual to formal (`Voice.PolishTone`).
const TONES: [&str; 5] = ["casual", "semi-casual", "balanced", "semi-formal", "formal"];
const DEFAULT_TONE: &str = "semi-formal";
/// Defaults when the setting is absent (new profiles); non-chat text still uses `DEFAULT_TONE`.
const NEW_TONE: &str = "balanced";
const NEW_WAIT_MS: u64 = 3_000;
/// S1-mini's other two options (V54, for testing): `Voice.PolishStructure`, `Voice.PolishContext`.
const STRUCTURES: [&str; 2] = ["prose", "lists"];
const CONTEXTS: [&str; 3] = ["auto", "general", "email"];
/// A background sentence may take longer: nobody is waiting for it.
const PREWARM_LIMIT: Duration = Duration::from_secs(5);
const SERVER_START: Duration = Duration::from_secs(90);
/// Shorter sentences ("Thanks.") have no grammar worth a model call.
const MIN_WORDS: usize = 5;
const CACHE_MAX: usize = 300;
/// Requests the server answers at once (`-np`): after release two sentences are asked together, and
/// one left over from a prewarm no longer holds up the release (sentences are short; 2048 tokens each).
const SLOTS: usize = 2;

const SYSTEM: &str = "You are a text normalizer for speech-to-text transcripts. The input begins with a control line specifying the styling, structure, and context settings; clean the transcript to match those settings and output only the cleaned text.";

/// The wait and the tone, structure and context from the last [`sync`]; every polish call reads them.
static WAIT_MS: AtomicU64 = AtomicU64::new(MIN_WAIT_MS);
static OPTIONS: Mutex<(&str, &str, &str)> = Mutex::new((DEFAULT_TONE, "prose", "auto"));

/// The wait limit in force now: the setting while plugged in; longer on battery, where the graphics
/// card and CPU are slowed (and longer again with Battery Saver), so the polish still finishes.
fn budget() -> Duration {
    let power = oasis_win::power::status();
    Duration::from_millis(scaled_wait(WAIT_MS.load(Ordering::SeqCst), power.on_battery, power.saver))
}

fn scaled_wait(base_ms: u64, on_battery: bool, saver: bool) -> u64 {
    let percent = match (on_battery, saver) {
        (false, false) => 100,
        (true, false) => 150,
        (_, true) => 200,
    };
    (base_ms * percent / 100).min(MAX_WAIT_MS)
}

/// The control line; context "auto" is email in a mail program and general elsewhere.
fn control(style: oasis_core::speech::style::Style) -> String {
    let (chosen, structure, context) = *lock(&OPTIONS);
    // The chosen tone is for chat, and for everyday apps when it is casual (`dictation_style`); mail,
    // notes and terminals stay semi-formal.
    let tone = if style == oasis_core::speech::style::Style::Message { chosen } else { DEFAULT_TONE };
    let context = match context {
        "auto" if style == oasis_core::speech::style::Style::Mail => "email",
        "auto" => "general",
        c => c,
    };
    format!("[Styling: {tone}] [Structure: {structure}] [Context: {context}]")
}

/// The tone picked in Settings is casual or semi-casual (from the last [`sync`]).
pub fn casual_tone() -> bool {
    matches!(lock(&OPTIONS).0, "casual" | "semi-casual")
}

/// `control` plus the user's own Dictionary words (those added with no "heard as"), so the model
/// spells them as given. Part of the control text, so the polish cache follows the Dictionary.
fn with_terms(control: String, glossary: &[oasis_core::speech::GlossaryEntry]) -> String {
    let mut terms = String::new();
    for g in glossary.iter().filter(|g| g.spoken == g.canonical) {
        if terms.len() + g.canonical.len() > 300 {
            break; // shortcut: a fixed cap keeps the prompt short; raise it if testers keep long lists
        }
        terms.push_str(if terms.is_empty() { "" } else { ", " });
        terms.push_str(&g.canonical);
    }
    if terms.is_empty() { control } else { format!("{control} [Terms: {terms}]") }
}

/// The known value of `Voice.<key>`, or the first one.
fn pick(voice: Option<&serde_json::Value>, key: &str, known: &[&'static str]) -> &'static str {
    voice.and_then(|v| v.get(key)).and_then(|v| v.as_str()).and_then(|t| known.iter().find(|k| **k == t)).copied().unwrap_or(known[0])
}

struct Running {
    child: Child,
    base: String,
    ready: Arc<AtomicBool>,
}

static SERVER: Mutex<Option<Running>> = Mutex::new(None);
/// Left-over servers from earlier runs are ended once, before the first start.
static STRAYS_ENDED: AtomicBool = AtomicBool::new(false);
static NOTE: Mutex<Option<String>> = Mutex::new(None);
static CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
/// When the model was last asked anything. Left idle, its next answer takes over a second (five
/// of five first dictations after a break ran out of the wait limit, 2026-11-13).
static LAST_ASK: Mutex<Option<Instant>> = Mutex::new(None);
const IDLE: Duration = Duration::from_secs(120);
/// After this long with no ask, prewarm or button press the server is stopped; it starts again on demand.
const IDLE_STOP: Duration = Duration::from_secs(10 * 60);
/// When polish was last used (ask, prewarm, button press or server start), for [`tick`].
static LAST_USE: Mutex<Option<Instant>> = Mutex::new(None);
/// Asks waiting for an answer now; the server is never stopped under one.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// The server was stopped for idleness and should start again at the next use.
static IDLE_STOPPED: AtomicBool = AtomicBool::new(false);
/// Where the models are, from the last [`sync`] that wanted polish; `None` when polish is off.
static MODELS: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Lines for the timing log (stop, restart), taken by [`tick`].
static LOG_LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// One background polish at a time: two at once asked for the same sentences twice, and the
/// release's own asks waited behind the copies.
static PREWARMING: Mutex<()> = Mutex::new(());
const WARM_UP: &str = "so um we should uh meet on tuesday";

/// Whether the model sat idle long enough to need a wake sentence (never asked counts as idle).
fn needs_wake(since_last_ask: Option<Duration>) -> bool {
    since_last_ask.is_none_or(|d| d >= IDLE)
}

/// Whether to stop the server to give back its ~1.2 GB: idle long enough, nothing being asked, the
/// talk button not held (owner's decision, 2026-11-16).
fn should_stop(idle: Duration, in_flight: bool, held: bool) -> bool {
    idle >= IDLE_STOP && !in_flight && !held
}

/// Round-trip split for the log: the server reports its own prompt + generation time, the rest of
/// the round trip was queue (and transport) time.
fn split_times(round_trip_ms: u64, reply: &serde_json::Value) -> (u64, u64) {
    let t = |k: &str| reply.pointer(&format!("/timings/{k}")).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let infer = ((t("prompt_ms") + t("predicted_ms")).round() as u64).min(round_trip_ms);
    (round_trip_ms - infer, infer)
}
/// Queue and inference milliseconds summed over the asks since the last [`apply`] note.
static QUEUE_MS: AtomicU64 = AtomicU64::new(0);
static INFER_MS: AtomicU64 = AtomicU64::new(0);

/// The talk button went down: if the model sat idle, ask it one short sentence now, so it is fast
/// again by the time the button is let go.
/// A server stopped for idleness starts again here; its start-up warm-up stands in for the wake sentence.
pub fn wake() {
    touch();
    if restart_if_stopped() {
        return;
    }
    let Some(base) = ready_base() else { return };
    if !needs_wake(lock(&LAST_ASK).map(|at| at.elapsed())) {
        return;
    }
    std::thread::spawn(move || {
        let _ = ask(&base, &control(oasis_core::speech::style::Style::Default), "", WARM_UP, PREWARM_LIMIT);
    });
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn touch() {
    *lock(&LAST_USE) = Some(Instant::now());
}

/// Start the server again if it was stopped for idleness. True when a start was begun. Never
/// blocks on the model loading: dictations until it is ready are typed as the rules wrote them.
fn restart_if_stopped() -> bool {
    let mut server = lock(&SERVER);
    if server.is_some() || !IDLE_STOPPED.swap(false, Ordering::SeqCst) {
        return false;
    }
    let Some(models) = lock(&MODELS).clone() else { return false };
    touch();
    start_locked(&mut server, &models, true);
    server.is_some()
}

/// Engine loop, every pass: stop a server left idle for [`IDLE_STOP`] (`held`: the talk button is
/// down or its recording is still being handled). Returns lines for the timing log.
pub fn tick(held: bool) -> Vec<String> {
    let mut server = lock(&SERVER);
    if server.is_some() {
        let idle = lock(&LAST_USE).map_or(Duration::ZERO, |at| at.elapsed());
        if should_stop(idle, IN_FLIGHT.load(Ordering::SeqCst) > 0, held) {
            stop_locked(&mut server);
            IDLE_STOPPED.store(true, Ordering::SeqCst);
            lock(&LOG_LINES).push(format!("polish server stopped after {} s idle (frees its RAM); starts again at the next talk-button press", idle.as_secs()));
        }
    }
    drop(server);
    std::mem::take(&mut *lock(&LOG_LINES))
}

/// Start or stop the polish server so it matches the settings. Cheap when nothing changed.
pub fn sync(models_dir: &Path, settings: &Settings) {
    let voice = settings.extra.get("Voice");
    // Only known values reach the control line, and the wait never goes under the minimum.
    let tone = voice.and_then(|v| v.get("PolishTone")).and_then(|v| v.as_str()).and_then(|t| TONES.iter().find(|k| **k == t)).copied().unwrap_or(NEW_TONE);
    // Absent structure means the new default (lists); an unknown saved value still falls back to prose.
    let structure = if voice.and_then(|v| v.get("PolishStructure")).is_none() { "lists" } else { pick(voice, "PolishStructure", &STRUCTURES) };
    let picked = (tone, structure, pick(voice, "PolishContext", &CONTEXTS));
    let wait = voice.and_then(|v| v.get("PolishWaitMs")).and_then(|v| v.as_u64()).unwrap_or(NEW_WAIT_MS).clamp(MIN_WAIT_MS, MAX_WAIT_MS);
    WAIT_MS.store(wait, Ordering::SeqCst);
    // The cache is keyed by the control line, so answers in other options are never reused.
    *lock(&OPTIONS) = picked;
    // The model manager sets PolishModel; the Voice screen's switch can turn it off (stops the server).
    let wanted = voice.and_then(|v| v.get("PolishModel")).and_then(|v| v.as_str()).is_some_and(|m| !m.is_empty())
        && voice.and_then(|v| v.get("PolishEnabled")).and_then(|v| v.as_bool()).unwrap_or(true);
    *lock(&MODELS) = wanted.then(|| models_dir.to_path_buf());
    let mut server = lock(&SERVER);
    if let Some(running) = server.as_mut() {
        // A server that died is dropped so the next sync starts a fresh one.
        if wanted && matches!(running.child.try_wait(), Ok(None)) {
            return;
        }
        stop_locked(&mut server);
    }
    // Stopped for idleness: a settings save does not bring it back, the next use does.
    if !wanted {
        IDLE_STOPPED.store(false, Ordering::SeqCst);
        return;
    }
    if IDLE_STOPPED.load(Ordering::SeqCst) {
        return;
    }
    touch();
    start_locked(&mut server, models_dir, false);
}

/// Spawn llama-server into `server` (nothing when its files are missing); a thread marks it ready
/// once loaded and warmed up. `restart`: log the time to ready.
fn start_locked(server: &mut Option<Running>, models_dir: &Path, restart: bool) {
    let runtime = models_dir.join("runtime").join("llama-vulkan");
    let gguf = models_dir.join("llm").join("s1-mini").join("s1-mini-q4_k_m.gguf");
    if !runtime.join("llama-server.exe").is_file() || !gguf.is_file() {
        return;
    }
    // Servers left by an OASIS that was killed before helpers were tied to it hold ~1.2 GB each.
    #[cfg(windows)]
    if !STRAYS_ENDED.swap(true, Ordering::SeqCst) {
        oasis_win::job::end_strays(&runtime.join("llama-server.exe"));
    }
    let Ok(port) = TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()) else { return };
    let mut command = Command::new(runtime.join("llama-server.exe"));
    command
        .arg("-m")
        .arg(&gguf)
        .args(["--host", "127.0.0.1", "--port", &port.to_string(), "-c", "4096", "-ngl", "99", "--jinja", "-np", &SLOTS.to_string(), "--no-webui"])
        // llama-server keeps every past prompt in RAM up to 8 GB by default (~3 MB per sentence
        // asked, never freed); OASIS caches its own answers, so that cache only piled up RAM.
        .args(["--cache-ram", "0"])
        .current_dir(&runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // no console window
    }
    let Ok(child) = command.spawn() else { return };
    #[cfg(windows)]
    oasis_win::job::end_with_this_process(child.id()); // never outlives OASIS, however it ends
    let ready = Arc::new(AtomicBool::new(false));
    let base = format!("http://127.0.0.1:{port}");
    *server = Some(Running { child, base: base.clone(), ready: Arc::clone(&ready) });
    // Wait for the model to load, then warm it up, off the engine's thread (it waits for the
    // caller to release the SERVER lock).
    std::thread::spawn(move || {
        let started = Instant::now();
        let agent = agent();
        while started.elapsed() < SERVER_START {
            if lock(&SERVER).as_ref().is_none_or(|r| r.base != base) {
                return; // stopped or replaced
            }
            if agent.get(&format!("{base}/health")).call().is_ok_and(|r| r.status().as_u16() == 200) {
                let _ = ask(&base, &control(oasis_core::speech::style::Style::Default), "", WARM_UP, PREWARM_LIMIT);
                ready.store(true, Ordering::SeqCst);
                if restart {
                    lock(&LOG_LINES).push(format!("polish server started again, ready in {} ms", started.elapsed().as_millis()));
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    });
}

/// The running server's process id, for the memory line in the timing log.
pub fn server_pid() -> Option<u32> {
    lock(&SERVER).as_ref().map(|r| r.child.id())
}

/// Stop the server (engine shutdown).
pub fn stop() {
    stop_locked(&mut lock(&SERVER));
    IDLE_STOPPED.store(false, Ordering::SeqCst);
}

fn stop_locked(server: &mut Option<Running>) {
    if let Some(mut running) = server.take() {
        let _ = running.child.kill();
        let _ = running.child.wait();
    }
    *lock(&CACHE) = None;
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).build().into()
}

fn ready_base() -> Option<String> {
    lock(&SERVER).as_ref().filter(|r| r.ready.load(Ordering::SeqCst)).map(|r| r.base.clone())
}

/// One sentence to the model; `None` on any failure or when `limit` runs out.
fn ask(base: &str, control: &str, before: &str, text: &str, limit: Duration) -> Option<String> {
    let asked = Instant::now();
    *lock(&LAST_ASK) = Some(asked);
    touch();
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    // Counted down however the ask ends (the `?`s below).
    struct Done;
    impl Drop for Done {
        fn drop(&mut self) {
            IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
            touch();
        }
    }
    let _done = Done;
    let body = serde_json::json!({
        "messages": [
            { "role": "system", "content": SYSTEM },
            { "role": "user", "content": if before.is_empty() { format!("{control}\n{text}") } else { format!("{control}\n(Context only, do not repeat or change: {before})\n{text}") } },
        ],
        "temperature": 0,
        "cache_prompt": true,
        "max_tokens": oasis_core::speech::polish_prompt::max_output_tokens(text.len()),
        "chat_template_kwargs": { "enable_thinking": false },
    });
    let mut response = agent()
        .post(&format!("{base}/v1/chat/completions"))
        .config()
        .timeout_global(Some(limit))
        .build()
        .content_type("application/json")
        .send(body.to_string().as_bytes())
        .ok()?;
    if response.status().as_u16() != 200 {
        return None;
    }
    let mut raw = String::new();
    response.body_mut().as_reader().read_to_string(&mut raw).ok()?;
    let reply: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let (queue, infer) = split_times(asked.elapsed().as_millis() as u64, &reply);
    QUEUE_MS.fetch_add(queue, Ordering::SeqCst);
    INFER_MS.fetch_add(infer, Ordering::SeqCst);
    let answer = reply.pointer("/choices/0/message/content")?.as_str()?;
    let answer = oasis_core::speech::polish_prompt::clean_local_answer(answer);
    (!answer.is_empty()).then_some(answer)
}

/// Sentences of one line, each keeping its closing mark. One over [`RUN_ON`] words is cut again
/// at the clause boundary nearest its middle, so the model gets pieces it can finish in time.
fn sentences<'a>(line: &'a str, keep: &[String]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = line.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if matches!(b, b'.' | b'?' | b'!') && bytes.get(i + 1).is_none_or(|n| *n == b' ') {
            split_run_on(line[start..=i].trim(), keep, &mut out);
            start = i + 1;
        }
    }
    if line[start..].trim().len() > 0 {
        split_run_on(line[start..].trim(), keep, &mut out);
    }
    out
}

/// Longest sentence sent to the model whole.
const RUN_ON: usize = 30;

fn split_run_on<'a>(s: &'a str, keep: &[String], out: &mut Vec<&'a str>) {
    match (s.split_whitespace().count() > RUN_ON).then(|| boundary(s, keep)).flatten() {
        Some(at) => {
            split_run_on(s[..at].trim(), keep, out);
            split_run_on(s[at..].trim(), keep, out);
        }
        None => out.push(s),
    }
}

/// Byte offset to cut at: after a comma, else before "but", "so", "because" or "and then", whichever
/// is nearest the middle (by words) and not inside a Dictionary term. A comma inside a number
/// ("1,000") is not followed by a space, so it never counts.
fn boundary(s: &str, keep: &[String]) -> Option<usize> {
    let mut terms = Vec::new();
    for t in keep.iter().filter(|t| !t.is_empty()) {
        terms.extend(s.match_indices(t.as_str()).map(|(i, m)| i..i + m.len()));
    }
    let bare = |w: &str| w.to_lowercase().trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    let all: Vec<&str> = s.split_whitespace().collect();
    let (mut commas, mut words) = (Vec::new(), Vec::new());
    for (n, w) in all.iter().enumerate() {
        let begin = w.as_ptr() as usize - s.as_ptr() as usize;
        let b = bare(w);
        let joiner = matches!(b.as_str(), "but" | "so" | "because") || (b == "and" && all.get(n + 1).is_some_and(|x| bare(x) == "then"));
        if n > 0 && joiner && !all[n - 1].ends_with(',') {
            words.push((n, begin));
        }
        if w.ends_with(',') && w.len() > 1 {
            commas.push((n + 1, begin + w.len()));
        }
    }
    let (total, middle) = (all.len(), all.len() / 2);
    let pool = if commas.is_empty() { words } else { commas };
    pool.into_iter()
        .filter(|(n, at)| *n >= 3 && total - *n >= 3 && !terms.iter().any(|r| r.start < *at && *at < r.end))
        .min_by_key(|(n, _)| n.abs_diff(middle))
        .map(|(_, at)| at)
}

/// A polished sentence is kept only if it keeps the meaning and every Dictionary term it had.
#[cfg(test)]
fn acceptable(sentence: &str, polished: &str, keep: &[String]) -> bool {
    rejection(sentence, polished, keep).is_none()
}

/// Why a polished sentence is thrown away (`growth`, `meaning`, `Dictionary`, `form`), or `None`.
fn rejection(sentence: &str, polished: &str, keep: &[String]) -> Option<&'static str> {
    let orig = sentence.split_whitespace().count();
    let grown = polished.split_whitespace().count() > oasis_core::speech::polish::max_polished_words(orig);
    // Form guard: no standalone lowercase "i", and a closing . ? ! the rules wrote stays.
    let ends = |s: &str| s.trim_end().ends_with(['.', '?', '!']);
    let form = !polished.split_whitespace().any(|t| t.trim_matches(|c: char| !c.is_alphanumeric()) == "i") && (!ends(sentence) || ends(polished));
    if grown {
        Some("growth")
    } else if !meaning::accepts(sentence, polished) {
        Some("meaning")
    } else if !keep.iter().all(|t| !sentence.contains(t.as_str()) || polished.contains(t.as_str())) {
        Some("Dictionary")
    } else if !form {
        Some("form")
    } else {
        None
    }
}

/// Rejected sentences since the last dictation was logged, as "reason: before -> after".
static REJECTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Takes the rejections for `dictation-trace.log`.
pub fn take_rejections() -> Vec<String> {
    std::mem::take(&mut *lock(&REJECTS))
}

/// `Some(polished)` when it passes; otherwise the reason is kept for the trace and `None` returned.
fn checked(sentence: &str, polished: String, keep: &[String]) -> Option<String> {
    match rejection(sentence, &polished, keep) {
        None => Some(polished),
        Some(why) => {
            lock(&REJECTS).push(format!("{why}: {sentence:?} -> {polished:?}"));
            None
        }
    }
}

/// The style of the program in front when the recording began, for [`prewarm`].
static FRONT_STYLE: Mutex<oasis_core::speech::style::Style> = Mutex::new(oasis_core::speech::style::Style::Default);

pub fn set_front_style(style: oasis_core::speech::style::Style) {
    *lock(&FRONT_STYLE) = style;
}

fn cached(key: &str) -> Option<String> {
    lock(&CACHE).as_ref().and_then(|c| c.get(key).cloned())
}

fn remember(key: String, polished: &str) {
    let mut cache = lock(&CACHE);
    let cache = cache.get_or_insert_with(HashMap::new);
    if cache.len() >= CACHE_MAX {
        cache.clear();
    }
    cache.insert(key, polished.to_string());
}

/// One sentence, polished or as it was.
/// `before` is the previous polished sentence, read-only context for the model.
fn polish_sentence(base: &str, control: &str, before: &str, sentence: &str, deadline: Instant, keep: &[String]) -> String {
    if sentence.split_whitespace().count() < MIN_WORDS {
        return sentence.to_string();
    }
    let key = format!("{control}|{sentence}");
    // A cached answer was checked with the Dictionary of the moment it was made; check it again.
    if let Some(done) = cached(&key) {
        return checked(sentence, done, keep).unwrap_or_else(|| sentence.to_string());
    }
    let Some(left) = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) else { return sentence.to_string() };
    // A rejected answer is remembered as the sentence itself: at temperature 0 the model would give
    // it again, and each prewarm and the release re-asked it (223 rejections, 32 sentences, 16 Nov).
    // No answer at all (time ran out) is not remembered, so it is asked again.
    let Some(answer) = ask(base, control, before, sentence, left) else { return sentence.to_string() };
    let out = checked(sentence, answer, keep).unwrap_or_else(|| sentence.to_string());
    remember(key, &out);
    out
}

/// Polish the rule-cleaned dictation, if the server is ready. Never slower than the wait limit.
pub fn apply(rules: CleanupResult, context: &CleanupContext, keep: &[String]) -> CleanupResult {
    let started = Instant::now();
    let Some(base) = ready_base() else {
        // Not waited for: this dictation keeps the rules' text, the next one is polished.
        let why = if restart_if_stopped() {
            "it was stopped after 10 idle minutes and is starting again"
        } else if lock(&SERVER).is_some() { "the model is still loading" } else { "it is off (switched off, or this PC has no polish model) or its files are missing" };
        *lock(&NOTE) = Some(format!("Dictation polish (S1-mini): not applied, {why}."));
        return rules;
    };
    let deadline = started + budget();
    let control = with_terms(control(context.style), &context.glossary);
    let mut unchanged = false;
    let mut llm = |text: &str, _raw: &str, _c: &CleanupContext| -> Option<String> {
        let lines: Vec<Vec<&str>> = text.split('\n').map(|line| sentences(line, keep)).collect();
        // (context, sentence): the context is the previous sentence as the rules wrote it, so the
        // sentences need not wait for each other and [`SLOTS`] of them are asked at once.
        let jobs: Vec<(&str, &str)> = lines.iter().flat_map(|parts| parts.iter().enumerate().map(|(i, s)| (if i == 0 { "" } else { parts[i - 1] }, *s))).collect();
        let done: Vec<Mutex<String>> = jobs.iter().map(|(_, s)| Mutex::new(s.to_string())).collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..SLOTS {
                scope.spawn(|| loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(&(before, s)) = jobs.get(i) else { break };
                    *lock(&done[i]) = polish_sentence(&base, &control, before, s, deadline, keep);
                });
            }
        });
        let mut polished = done.into_iter().map(|m| m.into_inner().unwrap_or_else(|p| p.into_inner()));
        let answer = lines.iter().map(|parts| parts.iter().filter_map(|_| polished.next()).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
        unchanged = answer == text;
        Some(answer)
    };
    let out = maybe_polish(rules, &mut llm, context);
    // One reason each, so the log says why a dictation came out as the rules wrote it.
    let how = if out.used_llm {
        "applied"
    } else if Instant::now() >= deadline {
        "not applied, rules kept: the wait limit ran out"
    } else if unchanged {
        "not applied: the model changed nothing"
    } else {
        "not applied, rules kept: too short, or the answer failed the meaning check"
    };
    *lock(&NOTE) = Some(format!("Dictation polish (S1-mini): {how} in {} ms for {} chars (model asks: queue {} ms, inference {} ms).", started.elapsed().as_millis(), out.corrected_text.len(), QUEUE_MS.swap(0, Ordering::SeqCst), INFER_MS.swap(0, Ordering::SeqCst)));
    out
}

/// What the last [`apply`] did, once, for the security log.
pub fn take_note() -> Option<String> {
    lock(&NOTE).take()
}

/// Dictation heard so far while the button is still held: polish every finished sentence now, in
/// the background, so the cache has them by release. The last sentence may still grow; it waits.
pub fn prewarm(raw_so_far: &str, settings: &Settings, paragraphs: &[String]) {
    let voice = oasis_settings::voice::VoiceSettings::from(settings);
    let style = *lock(&FRONT_STYLE);
    if !voice.dictation_cleanup || style == oasis_core::speech::style::Style::Exact {
        return;
    }
    touch();
    let Some(base) = ready_base() else {
        restart_if_stopped();
        return;
    };
    let raw = raw_so_far.to_string();
    let paragraphs = paragraphs.to_vec();
    std::thread::spawn(move || {
        let _one_at_a_time = lock(&PREWARMING);
        // Same input as `clean_dictation`: Dictionary, style, paragraphs and `keep`.
        let mut context = CleanupContext::dictate(true);
        context.style = style;
        context.glossary = voice.dictation_glossary.iter().map(|(s, c)| oasis_core::speech::GlossaryEntry::new(s, c)).collect();
        let keep: Vec<String> = voice.dictation_glossary.iter().map(|(_, c)| c.clone()).collect();
        let control = with_terms(control(style), &context.glossary);
        let text = cleanup_pipeline::run(&CleanupRequest::new(&raw, context)).corrected_text;
        let text = oasis_core::speech::layout::place_paragraphs(&text, &paragraphs);
        for line in text.split('\n') {
            let parts = sentences(line, &keep);
            // The sentence at the end of the text is probably unfinished.
            let done = if line == text.rsplit('\n').next().unwrap_or("") { parts.len().saturating_sub(1) } else { parts.len() };
            let mut before = String::new();
            for s in parts.into_iter().take(done) {
                before = polish_sentence(&base, &control, &before, s, Instant::now() + PREWARM_LIMIT, &keep);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_grows_on_battery_and_more_with_battery_saver() {
        assert_eq!(scaled_wait(600, false, false), 600, "plugged in: the setting as it is");
        assert_eq!(scaled_wait(600, true, false), 900);
        assert_eq!(scaled_wait(600, true, true), 1200);
        assert_eq!(scaled_wait(3000, true, true), MAX_WAIT_MS, "never past the ceiling");
    }

    #[test]
    fn the_model_stays_awake_for_two_minutes_and_queue_time_is_split_from_inference() {
        assert!(needs_wake(None));
        assert!(!needs_wake(Some(Duration::from_secs(60))));
        assert!(!needs_wake(Some(Duration::from_secs(119))));
        assert!(needs_wake(Some(Duration::from_secs(121))));
        let reply = serde_json::json!({ "timings": { "prompt_ms": 100.4, "predicted_ms": 200.0 } });
        assert_eq!(split_times(1000, &reply), (700, 300));
        assert_eq!(split_times(1000, &serde_json::json!({})), (1000, 0));
        assert_eq!(split_times(200, &reply), (0, 200));
    }

    #[test]
    fn the_server_stops_after_ten_idle_minutes_but_never_mid_ask_or_while_held() {
        assert!(!should_stop(Duration::from_secs(9 * 60), false, false));
        assert!(should_stop(Duration::from_secs(10 * 60), false, false));
        assert!(should_stop(Duration::from_secs(60 * 60), false, false));
        assert!(!should_stop(Duration::from_secs(60 * 60), true, false), "an ask is in flight");
        assert!(!should_stop(Duration::from_secs(60 * 60), false, true), "the talk button is held");
    }

    #[test]
    fn sentences_split_after_closing_marks_only() {
        assert_eq!(sentences("It costs 3.5 dollars. Is that fine? Yes", &[]), ["It costs 3.5 dollars.", "Is that fine?", "Yes"]);
        assert!(sentences("", &[]).is_empty());
    }

    #[test]
    fn a_run_on_sentence_splits_near_the_middle_but_not_inside_terms_or_numbers() {
        let w = |n: usize, tag: &str| (0..n).map(|i| format!("{tag}{i}")).collect::<Vec<_>>().join(" ");
        // 79 words, one comma after word 40.
        let long = format!("{}, {} {}.", w(40, "a"), w(38, "b"), "end");
        let parts = sentences(&long, &[]);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].ends_with(','));
        assert_eq!(parts.join(" "), long, "every word kept");
        assert!(parts.iter().all(|p| p.split_whitespace().count() <= 41));
        // No comma: split before "because" near the middle.
        let because = format!("{} because {}.", w(35, "a"), w(35, "b"));
        let parts = sentences(&because, &[]);
        assert_eq!(parts.len(), 2);
        assert!(parts[1].starts_with("because"));
        // A Dictionary term holding the only boundary word is never cut.
        let term = format!("{} Hit so Hard {}.", w(35, "a"), w(35, "b"));
        assert_eq!(sentences(&term, &["Hit so Hard".to_string()]).len(), 1);
        // A thousands separator is not a clause comma.
        let num = format!("{} 1,000 {}.", w(35, "a"), w(35, "b"));
        assert_eq!(sentences(&num, &[]).len(), 1);
        // Short sentences are untouched.
        assert_eq!(sentences("a, b so c.", &[]).len(), 1);
    }

    #[test]
    fn a_polished_sentence_must_keep_meaning_and_dictionary_terms() {
        let keep = vec!["OASIS".to_string()];
        assert!(acceptable("we shipped oasis today", "We shipped oasis today.", &[]));
        assert!(!acceptable("the OASIS board is ready", "The board is ready.", &keep));
        assert!(acceptable("the OASIS board is ready", "The OASIS board is ready.", &keep));
        assert!(!acceptable("call the bank", "Call the bank and then email the manager about it today.", &[]));
    }

    #[test]
    fn polish_that_loses_form_or_names_is_rejected() {
        assert!(!acceptable("I told Danosh about the plan today.", "I told danish about the plan today.", &[]));
        assert!(!acceptable("please push it to Git before lunch.", "Please push it to git before lunch.", &[]));
        assert!(!acceptable("so I wanted to explain the problem.", "So i wanted to explain the problem.", &[]));
        assert!(!acceptable("we met on tuesday about the new board.", "We met on Tuesday about the new board", &[]));
        assert!(acceptable("we met on tuesday about the new board.", "We met on Tuesday about the new board.", &[]));
    }

    #[test]
    fn a_grammar_fix_that_adds_four_words_is_accepted() {
        assert!(acceptable("make sure UART initialized before SPI starts", "Make sure the UART is initialized before the SPI bus starts", &[]));
    }

    #[test]
    fn a_cached_answer_missing_a_dictionary_term_is_rejected_and_logged() {
        let keep = vec!["KiCad".to_string()];
        let sentence = "we need the KiCad file before we start";
        let key = format!("c|{sentence}");
        remember(key.clone(), "We need the file before we start.");
        take_rejections();
        let out = polish_sentence("http://unused", "c", "", sentence, Instant::now(), &keep);
        assert_eq!(out, sentence);
        assert!(take_rejections().iter().any(|r| r.starts_with("Dictionary") || r.starts_with("meaning")));
    }

    /// A one-request stand-in for llama-server that answers `content`.
    fn one_answer_server(content: &str) -> String {
        use std::io::Write;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let body = serde_json::json!({ "choices": [{ "message": { "content": content } }] }).to_string();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            // Headers, then Content-Length bytes of body.
            while let Ok(n) = stream.read(&mut buf) {
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    if got.len() >= end + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        });
        base
    }

    #[test]
    fn a_rejected_answer_is_remembered_so_the_sentence_is_not_asked_again() {
        let sentence = "please send the quarterly report to the whole team";
        let base = one_answer_server("Please send the quarterly report to the whole team and also call everyone in the office tomorrow morning.");
        let out = polish_sentence(&base, "neg", "", sentence, Instant::now() + Duration::from_secs(5), &[]);
        assert_eq!(out, sentence, "growth: the rules stand");
        assert_eq!(cached(&format!("neg|{sentence}")).as_deref(), Some(sentence), "the answer the model gives at temperature 0 is not asked for twice");
    }

    /// Needs the QUALITY set downloaded and a graphics card: `cargo test -p oasis-engine polish -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_polish_on_the_graphics_card() {
        let models = Path::new(&std::env::var("LOCALAPPDATA").unwrap()).join("ProjectOASIS").join("Models");
        let settings: Settings = serde_json::from_value(serde_json::json!({ "Voice": { "PolishModel": "s1-mini" } })).unwrap();
        sync(&models, &settings);
        let started = Instant::now();
        while ready_base().is_none() {
            assert!(started.elapsed() < SERVER_START, "the server never became ready");
            std::thread::sleep(Duration::from_millis(200));
        }
        let ctx = CleanupContext::dictate(true);
        let raw = "so um i tested the new board this morning and the battery lasted almost twelve hours. then i called the supplier about the next order";
        prewarm(raw, &settings, &[]);
        std::thread::sleep(Duration::from_secs(2));
        let rules = cleanup_pipeline::run(&CleanupRequest::new(raw, ctx.clone()));
        let t = Instant::now();
        let out = apply(rules.clone(), &ctx, &[]);
        println!("rules : {}\npolish: {}\nafter release: {} ms, used_llm={}", rules.corrected_text, out.corrected_text, t.elapsed().as_millis(), out.used_llm);
        assert!(t.elapsed() < Duration::from_millis(900));
        stop();
    }

    #[test]
    fn the_control_line_takes_known_options_and_auto_follows_the_app() {
        use oasis_core::speech::style::Style;
        let settings: Settings = serde_json::from_value(serde_json::json!({ "Voice": { "PolishTone": "casual", "PolishStructure": "lists", "PolishContext": "auto" } })).unwrap();
        sync(Path::new("no-models-here"), &settings);
        assert_eq!(control(Style::Mail), "[Styling: semi-formal] [Structure: lists] [Context: email]");
        assert_eq!(control(Style::Message), "[Styling: casual] [Structure: lists] [Context: general]");
        let settings: Settings = serde_json::from_value(serde_json::json!({ "Voice": { "PolishStructure": "bullets", "PolishContext": "email" } })).unwrap();
        sync(Path::new("no-models-here"), &settings);
        assert_eq!(control(Style::Message), "[Styling: balanced] [Structure: prose] [Context: email]");
        // Nothing saved at all: the new defaults (balanced, lists, auto).
        sync(Path::new("no-models-here"), &serde_json::from_value(serde_json::json!({})).unwrap());
        assert_eq!(control(Style::Message), "[Styling: balanced] [Structure: lists] [Context: general]");
        assert_eq!(WAIT_MS.load(Ordering::SeqCst), 3_000);
    }

    #[test]
    fn without_a_server_the_rules_stand() {
        let ctx = CleanupContext::dictate(true);
        let rules = cleanup_pipeline::run(&CleanupRequest::new("send it today and then call me", ctx.clone()));
        assert_eq!(apply(rules.clone(), &ctx, &[]).corrected_text, rules.corrected_text);
    }
}
