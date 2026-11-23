//! Cheap audio rules for the microphone path: a noise gate and an energy-based silence detector.
//! Port of `NoiseGate.cs` and `EnergyVadPolicy.cs`. Core stays free of audio libraries; the
//! adapter feeds 16 kHz mono 16-bit little-endian samples and window peaks.

pub const DEFAULT_WINDOW_MS: usize = 20;
pub const DEFAULT_SPEECH_PEAK: i32 = 500;

pub const GATE_DEFAULT_THRESHOLD: i32 = 380;
pub const GATE_DEFAULT_PAD_MS: usize = 80;
pub const SAMPLE_RATE: usize = 16_000;
pub const BYTES_PER_SAMPLE: usize = 2;

/// Zero near-silent 20 ms windows except a small pad around speech. This is a noise gate, not
/// neural source separation. Audio with no speech at all is returned unchanged.
pub fn noise_gate(pcm16: &[u8], threshold: i32, pad_ms: usize) -> Vec<u8> {
    let window_bytes = SAMPLE_RATE / 50 * BYTES_PER_SAMPLE;
    if pcm16.len() < 4 || pcm16.len() < window_bytes {
        return pcm16.to_vec();
    }
    let windows = pcm16.len() / window_bytes;
    let speech: Vec<bool> =
        (0..windows).map(|i| peak(&pcm16[i * window_bytes..(i + 1) * window_bytes]) >= threshold).collect();

    let pad = pad_ms / DEFAULT_WINDOW_MS;
    let mut keep = vec![false; windows];
    for (i, _) in speech.iter().enumerate().filter(|(_, s)| **s) {
        for flag in &mut keep[i.saturating_sub(pad)..=(i + pad).min(windows - 1)] {
            *flag = true;
        }
    }
    if !keep.iter().any(|k| *k) {
        return pcm16.to_vec();
    }

    let mut output = pcm16.to_vec();
    for (i, _) in keep.iter().enumerate().filter(|(_, k)| !**k) {
        output[i * window_bytes..(i + 1) * window_bytes].fill(0);
    }
    if pcm16.len() > windows * window_bytes && !keep[windows - 1] {
        output[windows * window_bytes..].fill(0);
    }
    output
}

/// [`noise_gate`] with the default threshold and padding.
pub fn noise_gate_default(pcm16: &[u8]) -> Vec<u8> {
    noise_gate(pcm16, GATE_DEFAULT_THRESHOLD, GATE_DEFAULT_PAD_MS)
}

fn peak(buffer: &[u8]) -> i32 {
    buffer.chunks_exact(2).map(|b| i32::from(i16::from_le_bytes([b[0], b[1]])).abs()).max().unwrap_or(0)
}

/// Byte offset of the last speech end when trailing silence is long enough; otherwise `None`.
pub fn find_silence_cut_bytes(
    window_peaks: &[i32],
    window_bytes: usize,
    min_speech_bytes: usize,
    silence_windows_needed: usize,
    speech_threshold: i32,
) -> Option<usize> {
    if window_peaks.is_empty() || window_bytes == 0 || silence_windows_needed == 0 {
        return None;
    }
    let mut quiet_run = 0;
    let mut heard_speech = false;
    let mut last_speech_end = None;
    for (i, peak) in window_peaks.iter().enumerate() {
        let window_end = (i + 1) * window_bytes;
        if *peak >= speech_threshold {
            heard_speech = true;
            quiet_run = 0;
            last_speech_end = Some(window_end);
        } else if heard_speech {
            quiet_run += 1;
            if quiet_run >= silence_windows_needed {
                if let Some(end) = last_speech_end.filter(|e| *e >= min_speech_bytes) {
                    return Some(end);
                }
            }
        }
    }
    None
}

/// Forced-cut fallback for when nobody pauses long enough for [`find_silence_cut_bytes`]. Gaps
/// between words (50–150 ms) never reach the silence threshold but still show up as energy dips,
/// so cut in the quietest short span of the trailing search region. Spans are summed over a few
/// windows so a single quiet blip inside a word does not win; ties go to the later span.
/// Returns the byte offset at the middle of the quietest span (even), or `None`.
pub fn find_valley_cut_bytes(
    window_peaks: &[i32],
    window_bytes: usize,
    min_speech_bytes: usize,
    search_windows: usize,
    span_windows: usize,
) -> Option<usize> {
    if window_bytes == 0 || search_windows == 0 || span_windows == 0 || window_peaks.len() < span_windows {
        return None;
    }
    let half_span = span_windows * window_bytes / 2;
    let mut best: Option<(i64, usize)> = None;
    let mut start = window_peaks.len().saturating_sub(search_windows);
    while start + span_windows <= window_peaks.len() {
        let cut = start * window_bytes + half_span;
        if cut >= min_speech_bytes {
            let sum: i64 = window_peaks[start..start + span_windows].iter().map(|p| i64::from(*p)).sum();
            if best.is_none_or(|(s, _)| sum <= s) {
                best = Some((sum, cut));
            }
        }
        start += 1;
    }
    best.map(|(_, cut)| cut & !1)
}

/// Shortest pause that ends a segment while the talk button is held (`docs/VOICE_DECISIONS.md` V10).
/// Chosen by replaying 106 recordings (`replay_pauses` example): 850 ms/5 s keeps the wait after
/// release shortest that still reads well (last-piece decode 1.0 s average, 2.2 s p95, against
/// 1.1 s / 2.4 s at 1.2 s/8 s); the extra false breaks ("to. Use") are left to the dictation join.
pub const SEGMENT_PAUSE_MS: usize = 850;
/// Shortest segment worth decoding on its own: every decode costs about 0.3 s whatever its length,
/// so short pieces are left to grow until the next pause.
pub const MIN_SEGMENT_MS: usize = 5000;

/// Where to cut a recording still being made (16 kHz-style mono `samples` at `rate`): inside the
/// first pause of [`SEGMENT_PAUSE_MS`] after at least [`MIN_SEGMENT_MS`] of audio, a little after
/// the last word so its tail is kept. `None` while there is no such pause yet.
pub fn segment_cut(samples: &[f32], rate: u32) -> Option<usize> {
    cut_at_pause(samples, rate, SEGMENT_PAUSE_MS, MIN_SEGMENT_MS)
}

/// Pause that ends a piece of a spoken command (`docs/APP_WINDOWS.md` W11).
pub const COMMAND_PAUSE_MS: usize = 600;

/// Like [`segment_cut`] for a command: any length, a shorter pause. Commands are short, so by the
/// time the finger lifts the words are usually already decoded and nothing is left to wait for.
pub fn command_cut(samples: &[f32], rate: u32) -> Option<usize> {
    cut_at_pause(samples, rate, COMMAND_PAUSE_MS, 0)
}

/// Shared by [`segment_cut`], [`command_cut`] and the `replay_pauses` example.
pub fn cut_at_pause(samples: &[f32], rate: u32, pause_ms: usize, min_ms: usize) -> Option<usize> {
    let window = rate as usize * DEFAULT_WINDOW_MS / 1000;
    if window == 0 {
        return None;
    }
    // No hangover here: the pause itself is what is being measured.
    let peaks: Vec<i32> =
        speech_frames(samples, rate, 0).iter().map(|s| if *s { DEFAULT_SPEECH_PEAK } else { 0 }).collect();
    let pause_windows = pause_ms / DEFAULT_WINDOW_MS;
    let min = rate as usize * min_ms / 1000;
    let speech_end = find_silence_cut_bytes(&peaks, window, min, pause_windows, DEFAULT_SPEECH_PEAK)?;
    Some(speech_end + window * pause_windows / 2)
}

/// A pause this long between two pieces of dictation starts a new paragraph.
pub const PARAGRAPH_PAUSE_MS: usize = 2000;

/// Speech needs this many loud 20 ms frames in a row; one click or pop is never speech.
pub const SPEECH_START_FRAMES: usize = 3;
/// Frames still counted as speech after the last loud one (200 ms), so a dip between words holds.
pub const SPEECH_HANGOVER_FRAMES: usize = 10;
/// A frame is loud when its RMS (16-bit scale) is at least twice the clip's quiet floor, clamped to
/// this range. Calibrated on `DictationHistory`: silent recordings sit at an RMS floor of 140-200
/// with single frames up to ~350 (clicks reach 729); real words hold 400+ for 8 or more frames.
pub const SPEECH_RMS_MIN: f32 = 300.0;
pub const SPEECH_RMS_MAX: f32 = 600.0;

/// Frame loudness after the same boost [`normalize`] gives, so a quiet microphone is judged by the
/// thresholds below as a normal one is.
fn frame_rms(samples: &[f32], rate: u32) -> Vec<f32> {
    let window = (rate as usize * DEFAULT_WINDOW_MS / 1000).max(1);
    let gain = quiet_gain(samples);
    samples.chunks_exact(window).map(|w| (w.iter().map(|s| s * s).sum::<f32>() / window as f32).sqrt() * 32768.0 * gain).collect()
}

/// One flag per 20 ms frame: speech starts after [`SPEECH_START_FRAMES`] loud frames (flagged from
/// the first of them) and lasts `hangover` frames past the last loud one.
fn speech_frames(samples: &[f32], rate: u32, hangover: usize) -> Vec<bool> {
    let rms = frame_rms(samples, rate);
    let mut sorted = rms.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let floor = sorted.get(sorted.len() / 10).copied().unwrap_or(0.0); // 10th percentile
    let loud: Vec<bool> = rms.iter().map(|r| *r >= (floor * 2.0).clamp(SPEECH_RMS_MIN, SPEECH_RMS_MAX)).collect();
    let mut flags = vec![false; loud.len()];
    let mut i = 0;
    while i < loud.len() {
        if !loud[i] {
            i += 1;
            continue;
        }
        let end = i + loud[i..].iter().take_while(|l| **l).count();
        if end - i >= SPEECH_START_FRAMES {
            flags[i..(end + hangover).min(loud.len())].fill(true);
        }
        i = end;
    }
    flags
}

/// Whether `samples` hold anything loud enough, for long enough, to be speech.
pub fn has_speech(samples: &[f32], rate: u32) -> bool {
    speech_start(samples, rate).is_some()
}

/// Peak (16-bit scale), whole-clip RMS, frame count and speech frame count, for the log line of a
/// recording dropped as `no_speech`, so quiet speech that gets rejected can be found.
pub fn speech_stats(samples: &[f32], rate: u32) -> String {
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs())) * 32768.0;
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt() * 32768.0;
    let flags = speech_frames(samples, rate, SPEECH_HANGOVER_FRAMES);
    format!("peak={peak:.0} rms={rms:.0} frames={} speech_frames={}", flags.len(), flags.iter().filter(|f| **f).count())
}

/// Sample index where the first speech frame starts, if anywhere.
pub fn speech_start(samples: &[f32], rate: u32) -> Option<usize> {
    let window = (rate as usize * DEFAULT_WINDOW_MS / 1000).max(1);
    speech_frames(samples, rate, SPEECH_HANGOVER_FRAMES).iter().position(|s| *s).map(|f| f * window)
}

/// Quiet kept on each side of the speech when [`trim_to_speech`] cuts a clip (spec 1.3).
pub const TRIM_PAD_MS: usize = 160;

/// `samples` from [`TRIM_PAD_MS`] before the first speech frame to [`TRIM_PAD_MS`] after the last
/// one (hangover included). A clip with no speech comes back whole. Trimming, not gating: the
/// kept span is untouched.
pub fn trim_to_speech(samples: &[f32], rate: u32) -> &[f32] {
    let window = (rate as usize * DEFAULT_WINDOW_MS / 1000).max(1);
    let flags = speech_frames(samples, rate, SPEECH_HANGOVER_FRAMES);
    let (Some(first), Some(last)) = (flags.iter().position(|f| *f), flags.iter().rposition(|f| *f)) else { return samples };
    let pad = rate as usize * TRIM_PAD_MS / 1000;
    &samples[(first * window).saturating_sub(pad)..((last + 1) * window + pad).min(samples.len())]
}

/// Raises a quiet recording (a laptop's own microphone) to a normal speaking level. The level is
/// the 99.5th percentile of the samples, not the peak, so one click or pop cannot hold the gain
/// down. Never lowers it and never raises it past `QUIET_MAX_GAIN` times, so near-silence is not
/// blown up into noise. Loud moments that overshoot are soft-limited, not cut flat.
pub fn normalize(samples: &mut [f32]) {
    let gain = quiet_gain(samples);
    if gain > 1.0 {
        samples.iter_mut().for_each(|s| *s = soft_limit(*s * gain));
    }
}

/// The gain [`normalize`] applies: 1.0 when the clip is loud enough or only room noise.
fn quiet_gain(samples: &[f32]) -> f32 {
    let mut buckets = [0u32; 1024];
    for s in samples {
        buckets[((s.abs() * 1024.0) as usize).min(1023)] += 1;
    }
    let keep = samples.len() - samples.len() / 200; // up to the 99.5th percentile
    let (mut seen, mut level) = (0usize, 1.0f32);
    for (i, n) in buckets.iter().enumerate() {
        seen += *n as usize;
        if seen >= keep {
            level = (i + 1) as f32 / 1024.0;
            break;
        }
    }
    if samples.is_empty() || level < QUIET_MIN_LEVEL {
        return 1.0;
    }
    let gain = (QUIET_TARGET_LEVEL / level).min(QUIET_MAX_GAIN);
    if gain > 1.2 { gain } else { 1.0 }
}

fn soft_limit(s: f32) -> f32 {
    const KNEE: f32 = 0.8;
    if s.abs() <= KNEE {
        s
    } else {
        s.signum() * (KNEE + (1.0 - KNEE) * ((s.abs() - KNEE) / (1.0 - KNEE)).tanh())
    }
}
const QUIET_TARGET_LEVEL: f32 = 0.5;
const QUIET_MAX_GAIN: f32 = 8.0;
const QUIET_MIN_LEVEL: f32 = 0.01; // below this it is room noise, not a voice

/// Whether a piece that opens with `samples` follows a pause long enough for a new paragraph.
/// The piece before it ended [`SEGMENT_PAUSE_MS`]` / 2` into the pause ([`segment_cut`]).
pub fn starts_paragraph(samples: &[f32], rate: u32) -> bool {
    let quiet = speech_start(samples, rate).unwrap_or(samples.len());
    let ms = quiet * 1000 / (rate.max(1) as usize) + SEGMENT_PAUSE_MS / 2;
    ms >= PARAGRAPH_PAUSE_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_raises_quiet_audio_only() {
        let level = |v: &[f32]| v.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        // Speech-like: mostly a 0.05 tone, one 0.9 click that must not hold the gain down.
        let mut quiet: Vec<f32> = (0..16_000).map(|i| 0.05 * (i as f32 * 0.3).sin()).collect();
        quiet[100] = 0.9;
        normalize(&mut quiet);
        assert!(quiet[1000..16_000].iter().fold(0.0f32, |m, s| m.max(s.abs())) > 0.3);
        assert!(level(&quiet) <= 1.0);
        let mut loud = [0.6, -0.5];
        normalize(&mut loud);
        assert_eq!(loud, [0.6, -0.5]);
        let mut hiss = [0.005, -0.004];
        normalize(&mut hiss);
        assert_eq!(hiss, [0.005, -0.004]);
    }

    /// `parts` of (milliseconds, loud) at 16 kHz.
    fn talk(parts: &[(usize, bool)]) -> Vec<f32> {
        parts.iter().flat_map(|(ms, loud)| std::iter::repeat_n(if *loud { 0.3 } else { 0.001 }, ms * 16)).collect()
    }

    #[test]
    fn a_segment_ends_inside_the_first_long_enough_pause_after_five_seconds() {
        // A pause at 1 s is too early; the pause after 6.5 s of audio ends the segment, 420 ms in
        // (half of the 850 ms pause, rounded down to 20 ms windows).
        let audio = talk(&[(1000, true), (1000, false), (4500, true), (1000, false), (1000, true)]);
        assert_eq!(segment_cut(&audio, 16_000), Some(6500 * 16 + 420 * 16));
    }

    #[test]
    fn no_cut_without_a_long_enough_pause() {
        assert_eq!(segment_cut(&talk(&[(6000, true), (800, false), (1000, true)]), 16_000), None);
        assert_eq!(segment_cut(&talk(&[(2000, true), (1500, false)]), 16_000), None, "too short to stand alone");
        assert_eq!(segment_cut(&talk(&[(5000, false)]), 16_000), None, "no speech at all");
    }

    #[test]
    fn a_command_is_cut_at_its_first_short_pause_whatever_its_length() {
        // 1 s of words, then 700 ms quiet: cut inside the pause. Dictation would still wait.
        let audio = talk(&[(1000, true), (700, false)]);
        assert_eq!(command_cut(&audio, 16_000), Some(1000 * 16 + 300 * 16));
        assert_eq!(segment_cut(&audio, 16_000), None);
        assert_eq!(command_cut(&talk(&[(1000, true), (500, false)]), 16_000), None, "pause too short");
        assert_eq!(command_cut(&talk(&[(600, false), (500, true)]), 16_000), None, "quiet before speech is not a cut");
    }

    #[test]
    fn a_long_pause_before_a_piece_starts_a_paragraph() {
        assert!(starts_paragraph(&talk(&[(1800, false), (500, true)]), 16_000));
        assert!(!starts_paragraph(&talk(&[(600, false), (500, true)]), 16_000));
    }

    #[test]
    fn trim_keeps_the_speech_and_160_ms_of_quiet_each_side() {
        // 1 s quiet, 500 ms talk, 1 s quiet: speech 1000..1500 ms, hangover to 1700 ms, pad 160 ms.
        let audio = talk(&[(1000, false), (500, true), (1000, false)]);
        let kept = trim_to_speech(&audio, 16_000);
        assert_eq!(kept.len(), (1700 + 160 - (1000 - 160)) * 16);
        assert_eq!(kept.as_ptr(), audio[(1000 - 160) * 16..].as_ptr());
    }

    #[test]
    fn trim_leaves_silence_whole_and_stops_at_the_edges() {
        let quiet = talk(&[(800, false)]);
        assert_eq!(trim_to_speech(&quiet, 16_000).len(), quiet.len(), "no speech: nothing cut");
        let edges = talk(&[(500, true), (100, false), (500, true)]);
        assert_eq!(trim_to_speech(&edges, 16_000).len(), edges.len(), "speech touching both ends");
        assert_eq!(trim_to_speech(&[], 16_000).len(), 0);
    }

    #[test]
    fn speech_is_told_from_quiet() {
        assert!(has_speech(&talk(&[(100, false), (60, true)]), 16_000));
        assert!(!has_speech(&talk(&[(500, false)]), 16_000));
    }

    #[test]
    fn one_click_or_one_loud_frame_is_not_speech() {
        // A single 655 sample (the old threshold's trigger) and a 40 ms burst both stay below 3 frames.
        let mut click = talk(&[(500, false)]);
        click[4000] = 655.0 / 32768.0;
        assert!(!has_speech(&click, 16_000));
        assert!(!has_speech(&talk(&[(300, false), (40, true), (300, false)]), 16_000));
        assert_eq!(speech_start(&talk(&[(100, false), (60, true)]), 16_000), Some(100 * 16));
    }

    #[test]
    fn a_dip_inside_speech_is_bridged_by_the_hangover() {
        let flags = speech_frames(&talk(&[(100, true), (100, false), (100, true), (400, false)]), 16_000, SPEECH_HANGOVER_FRAMES);
        assert!(flags[..10].iter().all(|f| *f) && flags[10..15].iter().all(|f| *f), "dip held");
        assert!(!flags[flags.len() - 1], "trailing quiet is not speech");
    }

    fn pcm(windows: &[i16]) -> Vec<u8> {
        let mut out = Vec::new();
        for level in windows {
            for _ in 0..640 / 2 {
                out.extend_from_slice(&level.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn gate_silences_far_windows_and_keeps_the_pad() {
        // 10 windows; speech in window 5; pad of 80 ms = 4 windows keeps 1..=9, only window 0 is cut.
        let mut levels = vec![100i16; 10];
        levels[5] = 2000;
        let out = noise_gate(&pcm(&levels), 380, 80);
        assert!(out[..640].iter().all(|b| *b == 0));
        assert_eq!(out[640..], pcm(&levels)[640..]);
        // With no pad only the speech window survives.
        let tight = noise_gate(&pcm(&levels), 380, 0);
        assert!(tight[..5 * 640].iter().all(|b| *b == 0));
        assert_eq!(tight[5 * 640..6 * 640], pcm(&levels)[5 * 640..6 * 640]);
    }

    #[test]
    fn gate_leaves_all_quiet_audio_alone() {
        let quiet = pcm(&[10; 6]);
        assert_eq!(noise_gate_default(&quiet), quiet);
        assert_eq!(noise_gate_default(&[1, 2]), vec![1, 2]);
    }

    #[test]
    fn silence_cut_waits_for_enough_quiet() {
        let peaks = [900, 900, 100, 100, 100];
        assert_eq!(find_silence_cut_bytes(&peaks, 640, 0, 3, 500), Some(2 * 640));
        assert_eq!(find_silence_cut_bytes(&peaks, 640, 0, 4, 500), None);
        assert_eq!(find_silence_cut_bytes(&peaks, 640, 5000, 3, 500), None);
        assert_eq!(find_silence_cut_bytes(&[], 640, 0, 3, 500), None);
    }

    #[test]
    fn valley_cut_picks_the_quietest_late_span() {
        let peaks = [900, 900, 50, 40, 60, 900, 900];
        let cut = find_valley_cut_bytes(&peaks, 640, 0, 7, 3).unwrap();
        assert_eq!(cut, 2 * 640 + 960);
        assert_eq!(cut % 2, 0);
        assert_eq!(find_valley_cut_bytes(&peaks, 640, 1_000_000, 7, 3), None);
        assert_eq!(find_valley_cut_bytes(&[1], 640, 0, 7, 3), None);
    }
}
