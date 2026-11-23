//! Short sounds (the start and end clicks) on a chosen speaker. `PlaySound` can only use the
//! system's default output, which a USB microphone with its own (empty) speaker can take over.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};

/// The names of the speakers Windows offers.
pub fn output_names() -> Vec<String> {
    cpal::default_host().output_devices().map(|d| d.filter_map(|d| d.name().ok()).collect()).unwrap_or_default()
}

/// What the open stream is playing: interleaved samples already at the speaker's rate and channels.
struct Playing {
    samples: Vec<f32>,
    pos: usize,
}

/// A stream kept open on one speaker. Opening takes about 100 ms, so it is opened by the quiet
/// "wake" cue at the press (or the first cue) and kept for 30 s: the start click then only has to
/// hand over its samples, and sounds at once. It also keeps a sleeping speaker awake.
struct Open {
    name: String,
    _stream: cpal::Stream,
    playing: Arc<Mutex<Playing>>,
    dead: Arc<AtomicBool>,
    channels: usize,
    rate: u32,
}

/// Plays `wav` (16-bit PCM) on the speaker named `device` and returns at once: listing the speakers
/// takes tens of milliseconds, so it happens on the worker, never on the caller's thread. When it
/// cannot be started (no such speaker, odd file) the worker calls `fallback`, which plays it on the
/// system default. A newer cue replaces one still sounding.
pub fn play(wav: &'static [u8], device: &str, fallback: fn(&'static [u8])) {
    static WORKER: OnceLock<Sender<(&'static [u8], String, fn(&'static [u8]))>> = OnceLock::new();
    let tx = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<(&'static [u8], String, fn(&'static [u8]))>();
        let _ = std::thread::Builder::new().name("cue-out".into()).spawn(move || {
            let mut open: Option<Open> = None;
            loop {
                match rx.recv_timeout(Duration::from_secs(30)) {
                    Ok(mut job) => {
                        while let Ok(newer) = rx.try_recv() {
                            job = newer;
                        }
                        if open.as_ref().is_none_or(|o| o.name != job.1 || o.dead.load(Ordering::Relaxed)) {
                            open = open_stream(&job.1);
                        }
                        match (&open, decode(job.0)) {
                            (Some(o), Some((samples, channels, rate))) => {
                                let converted = convert(&samples, channels, rate, o.channels, o.rate);
                                if let Ok(mut p) = o.playing.lock() {
                                    *p = Playing { samples: converted, pos: 0 };
                                }
                            }
                            _ => (job.2)(job.0),
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => open = None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        tx
    });
    if tx.send((wav, device.to_string(), fallback)).is_err() {
        fallback(wav);
    }
}

fn find(name: &str) -> Option<cpal::Device> {
    let wanted = name.trim().to_lowercase();
    if wanted.is_empty() {
        return None;
    }
    cpal::default_host().output_devices().ok()?.find(|d| d.name().is_ok_and(|n| n.to_lowercase() == wanted))
}

/// Interleaved samples, channels and rate of a 16-bit PCM WAV.
fn decode(wav: &[u8]) -> Option<(Vec<f32>, usize, u32)> {
    if wav.len() < 44 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let (mut pos, mut fmt, mut data) = (12, None, None);
    while pos + 8 <= wav.len() {
        let size = u32::from_le_bytes(wav[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = wav.get(pos + 8..(pos + 8 + size).min(wav.len()))?;
        match &wav[pos..pos + 4] {
            b"fmt " if body.len() >= 16 => fmt = Some((u16::from_le_bytes([body[0], body[1]]), u16::from_le_bytes([body[2], body[3]]), u32::from_le_bytes(body[4..8].try_into().ok()?), u16::from_le_bytes([body[14], body[15]]))),
            b"data" => data = Some(body),
            _ => {}
        }
        pos += 8 + size + (size & 1);
    }
    let (format, channels, rate, bits) = fmt?;
    if format != 1 || bits != 16 || channels == 0 {
        return None;
    }
    Some((data?.chunks_exact(2).map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0).collect(), usize::from(channels), rate))
}

/// The samples at the speaker's rate and channel count (nearest sample: fine for a click).
fn convert(samples: &[f32], channels: usize, rate: u32, out_channels: usize, out_rate: u32) -> Vec<f32> {
    let frames = samples.len() / channels;
    let total = (frames as f64 * f64::from(out_rate) / f64::from(rate)) as usize;
    let mut out = Vec::with_capacity(total * out_channels);
    for frame in 0..total {
        let src = ((frame as f64 * f64::from(rate) / f64::from(out_rate)) as usize).min(frames.saturating_sub(1));
        for ch in 0..out_channels {
            out.push(samples.get(src * channels + ch.min(channels - 1)).copied().unwrap_or(0.0));
        }
    }
    out
}

fn open_stream(name: &str) -> Option<Open> {
    let device = find(name)?;
    let default = device.default_output_config().ok()?;
    let (channels, rate) = (usize::from(default.channels()), default.sample_rate().0);
    let playing = Arc::new(Mutex::new(Playing { samples: Vec::new(), pos: 0 }));
    let dead = Arc::new(AtomicBool::new(false));
    let fill = {
        let playing = playing.clone();
        move |out: &mut [f32]| {
            let Ok(mut p) = playing.lock() else { return out.fill(0.0) };
            for s in out.iter_mut() {
                *s = p.samples.get(p.pos).copied().unwrap_or(0.0);
                p.pos += 1;
            }
            if p.pos >= p.samples.len() {
                p.samples.clear();
                p.pos = 0;
            }
        }
    };
    let config: StreamConfig = default.clone().into();
    let err = { let dead = dead.clone(); move |_| dead.store(true, Ordering::Relaxed) };
    let stream = match default.sample_format() {
        SampleFormat::F32 => device.build_output_stream(&config, move |o: &mut [f32], _| fill(o), err, None),
        SampleFormat::I16 => {
            let mut buf = Vec::new();
            device.build_output_stream(&config, move |o: &mut [i16], _| {
                buf.resize(o.len(), 0.0);
                fill(&mut buf);
                for (d, s) in o.iter_mut().zip(&buf) {
                    *d = (s * 32767.0) as i16;
                }
            }, err, None)
        }
        _ => return None,
    }
    .ok()?;
    stream.play().ok()?;
    Some(Open { name: name.to_string(), _stream: stream, playing, dead, channels, rate })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_cue_files_decode_and_odd_input_does_not() {
        for wav in [&include_bytes!("../../oasis-engine/assets/voice-in.wav")[..], &include_bytes!("../../oasis-engine/assets/voice-out.wav")[..]] {
            let (samples, channels, rate) = super::decode(wav).expect("cue decodes");
            assert!(!samples.is_empty() && channels >= 1 && rate >= 8000);
        }
        assert!(super::decode(b"nope").is_none());
        assert!(super::find("no such speaker").is_none());
    }
}
