//! The microphone. Port of `Services/Voice/MicrophoneCapture.cs` and `MicrophoneDevices.cs`
//! (capture, device choice by name; the level meter and the pre-roll rule come with the host).
//!
//! `cpal`'s stream is not `Send` everywhere, so the stream lives on its own thread and this
//! handle talks to it. Audio is only kept between `start` and `stop`.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use oasis_models::AudioClip;

use crate::resample::{level, to_model_rate_at, to_mono};

/// The names of the microphones Windows offers.
pub fn device_names() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// The device to use: the one whose name matches `wanted` (C# saved a possibly shortened name,
/// so a prefix or containing match counts), else the default input. `true` with the named one.
fn pick_device(wanted: &str) -> Option<(cpal::Device, bool)> {
    let host = cpal::default_host();
    let wanted = wanted.trim().to_lowercase();
    if !wanted.is_empty() {
        if let Ok(devices) = host.input_devices() {
            let found = devices.into_iter().find(|d| {
                d.name().is_ok_and(|n| {
                    let n = n.to_lowercase();
                    n == wanted || n.starts_with(&wanted) || wanted.starts_with(&n) || n.contains(&wanted)
                })
            });
            if let Some(found) = found {
                return Some((found, true));
            }
        }
    }
    host.default_input_device().map(|d| (d, false))
}

enum Command {
    Start(Sender<Result<(), String>>),
    Stop(Sender<AudioClip>),
    Take(Sender<AudioClip>),
    Quit,
}

pub struct Microphone {
    commands: Sender<Command>,
    level: Arc<Mutex<f32>>,
    thread: Option<JoinHandle<()>>,
}

impl Microphone {
    /// Prepare the microphone named `device_name` (empty = the default). Nothing is recorded
    /// until [`Microphone::start`].
    pub fn new(device_name: &str) -> Microphone {
        let (commands, inbox) = mpsc::channel();
        let level = Arc::new(Mutex::new(0.0));
        let name = device_name.to_string();
        let shared_level = Arc::clone(&level);
        let thread = thread::Builder::new()
            .name("oasis-microphone".into())
            .spawn(move || audio_thread(&name, &inbox, &shared_level))
            .ok();
        Microphone { commands, level, thread }
    }

    /// Start recording. Fails with a sentence for the user if there is no microphone or it cannot
    /// be opened.
    pub fn start(&self) -> Result<(), String> {
        let (reply, answer) = mpsc::channel();
        self.commands.send(Command::Start(reply)).map_err(|_| "The microphone is not available.".to_string())?;
        answer.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| Err("The microphone did not start in time.".into()))
    }

    /// Stop recording and return what was heard, as 16 kHz mono.
    pub fn stop(&self) -> AudioClip {
        let (reply, answer) = mpsc::channel();
        if self.commands.send(Command::Stop(reply)).is_err() {
            return empty();
        }
        answer.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| empty())
    }

    /// What was heard since `start` or the last `take`, as 16 kHz mono, while recording goes on.
    /// `stop` then returns only what came after.
    pub fn take(&self) -> AudioClip {
        let (reply, answer) = mpsc::channel();
        if self.commands.send(Command::Take(reply)).is_err() {
            return empty();
        }
        answer.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| empty())
    }

    /// How loud it is right now, 0.0 to 1.0.
    pub fn level(&self) -> f32 {
        self.level.lock().map(|l| *l).unwrap_or(0.0)
    }
}

impl Drop for Microphone {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn empty() -> AudioClip {
    AudioClip { samples: Vec::new(), sample_rate: crate::resample::MODEL_RATE }
}

/// Resample what `buffer` holds for a `take`, keeping the source samples the last output sample
/// did not consume (and the fraction in `phase`) so the next take continues without a gap.
fn take_resampled(buffer: &mut Vec<f32>, rate: u32, phase: &mut f64) -> Vec<f32> {
    let (out, next) = to_model_rate_at(buffer, rate, *phase);
    let whole = (next.floor() as usize).min(buffer.len());
    buffer.drain(..whole);
    *phase = next - whole as f64;
    out
}

fn audio_thread(device_name: &str, inbox: &Receiver<Command>, level_out: &Arc<Mutex<f32>>) {
    let recorded: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let mut phase = 0.0f64; // where the next 16 kHz sample starts in `recorded`, in source samples
    let mut stream: Option<(cpal::Stream, u32)> = None;
    let mut known = None;

    while let Ok(command) = inbox.recv() {
        match command {
            Command::Start(reply) => {
                drop(stream.take());
                phase = 0.0;
                if let Ok(mut buffer) = recorded.lock() {
                    buffer.clear();
                }
                let result = open(device_name, &mut known, &recorded, level_out).map(|opened| stream = Some(opened));
                let _ = reply.send(result);
            }
            Command::Stop(reply) => {
                let rate = stream.as_ref().map_or(crate::resample::MODEL_RATE, |(_, rate)| *rate);
                drop(stream.take());
                let samples = recorded.lock().map(|mut b| std::mem::take(&mut *b)).unwrap_or_default();
                if let Ok(mut l) = level_out.lock() {
                    *l = 0.0;
                }
                let _ = reply.send(AudioClip {
                    samples: to_model_rate_at(&samples, rate, phase).0,
                    sample_rate: crate::resample::MODEL_RATE,
                });
                phase = 0.0;
            }
            Command::Take(reply) => {
                let rate = stream.as_ref().map_or(crate::resample::MODEL_RATE, |(_, rate)| *rate);
                let samples = recorded.lock().map(|mut b| take_resampled(&mut b, rate, &mut phase)).unwrap_or_default();
                let _ = reply.send(AudioClip { samples, sample_rate: crate::resample::MODEL_RATE });
            }
            Command::Quit => return,
        }
    }
}

/// Open the device. Its default format is tried first; if Windows refuses it (some USB microphones
/// do, even though they offered it), every other format the device lists is tried before giving up.
///
/// This runs on every press of the talk button, so the device and the format that opened are kept
/// in `known` for the next press: finding them again and opening a fresh device cost 0.25 to 1.8 s
/// a press for a USB webcam mic, against about 45 ms reused (measured 2026-11-16), all of it before
/// the start sound. With no device named, the Windows default is asked each time (a few ms) and the
/// kept one is used only while it is still the default.
fn open(
    device_name: &str,
    known: &mut Option<(cpal::Device, cpal::SupportedStreamConfig)>,
    recorded: &Arc<Mutex<Vec<f32>>>,
    level_out: &Arc<Mutex<f32>>,
) -> Result<(cpal::Stream, u32), String> {
    let by_default = device_name.trim().is_empty();
    if let Some((device, config)) = known.as_ref() {
        let still = !by_default || cpal::default_host().default_input_device().and_then(|d| d.name().ok()) == device.name().ok();
        if still {
            if let Ok(opened) = open_with(device, config, recorded, level_out) {
                return Ok(opened);
            }
        }
    }
    *known = None; // unplugged, refused or no longer the default: look again
    let (device, named) = pick_device(device_name).ok_or("No microphone was found.")?;
    // shortcut: a named mic that is missing is looked for on every press (about 0.3 s), so it is used once plugged in
    let keep = |config: &cpal::SupportedStreamConfig, known: &mut Option<_>| {
        if named || by_default {
            *known = Some((device.clone(), config.clone()));
        }
    };
    let mut last_error = String::from("The microphone cannot be used.");
    let default = device.default_input_config().ok();
    if let Some(config) = &default {
        match open_with(&device, config, recorded, level_out) {
            Ok(opened) => {
                keep(config, known);
                return Ok(opened);
            }
            Err(why) => last_error = why,
        }
    }
    // Listing every format is slow (about 0.25 s), so only when the default was refused.
    for config in device.supported_input_configs().into_iter().flatten().map(cpal::SupportedStreamConfigRange::with_max_sample_rate) {
        if Some(&config) == default.as_ref() {
            continue;
        }
        match open_with(&device, &config, recorded, level_out) {
            Ok(opened) => {
                keep(&config, known);
                return Ok(opened);
            }
            Err(why) => last_error = why,
        }
    }
    Err(last_error)
}

/// The meter reading for a block `now` when the recent loudest block was `loudest`: a quiet
/// microphone is lifted (at most 8x) so speech swings the pill as much as on an external one.
/// Room noise stays low because `loudest` below 0.02 is not lifted further.
fn meter_level(now: f32, loudest: f32) -> f32 {
    let gain = (0.4 / loudest.max(0.02)).clamp(1.0, 8.0);
    (now * gain).min(1.0)
}

fn open_with(
    device: &cpal::Device,
    supported: &cpal::SupportedStreamConfig,
    recorded: &Arc<Mutex<Vec<f32>>>,
    level_out: &Arc<Mutex<f32>>,
) -> Result<(cpal::Stream, u32), String> {
    let (rate, channels, format) = (supported.sample_rate().0, usize::from(supported.channels()), supported.sample_format());
    let config: cpal::StreamConfig = supported.clone().into();

    let sink = Arc::clone(recorded);
    let meter = Arc::clone(level_out);
    let mut loudest = 0.0f32; // the recent loudest block, so a quiet microphone's meter moves like a loud one's
    let mut push = move |frames: Vec<f32>| {
        let mono = to_mono(&frames, channels);
        let now = level(&mono);
        loudest = now.max(loudest * 0.995);
        if let Ok(mut l) = meter.lock() {
            *l = meter_level(now, loudest);
        }
        if let Ok(mut buffer) = sink.lock() {
            buffer.extend_from_slice(&mono);
        }
    };
    let failed = |e: cpal::StreamError| eprintln!("microphone error: {e}");

    let stream = match format {
        cpal::SampleFormat::F32 => device.build_input_stream(&config, move |data: &[f32], _| push(data.to_vec()), failed, None),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| push(data.iter().map(|s| f32::from(*s) / 32768.0).collect()),
            failed,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _| push(data.iter().map(|s| (f32::from(*s) - 32768.0) / 32768.0).collect()),
            failed,
            None,
        ),
        cpal::SampleFormat::I32 => device.build_input_stream(
            &config,
            move |data: &[i32], _| push(data.iter().map(|s| *s as f32 / 2_147_483_648.0).collect()),
            failed,
            None,
        ),
        cpal::SampleFormat::U8 => device.build_input_stream(
            &config,
            move |data: &[u8], _| push(data.iter().map(|s| (f32::from(*s) - 128.0) / 128.0).collect()),
            failed,
            None,
        ),
        other => return Err(format!("The microphone's audio format ({other:?}) is not supported.")),
    }
    .map_err(|e| format!("The microphone could not be opened: {e}"))?;
    stream.play().map_err(|e| format!("The microphone could not start: {e}"))?;
    Ok((stream, rate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resample::to_model_rate;

    #[test]
    fn a_quiet_microphone_moves_the_meter_like_a_loud_one() {
        assert!((meter_level(0.05, 0.05) - 0.4).abs() < 1e-4); // quiet speech peak reads 0.4
        assert_eq!(meter_level(0.5, 0.5), 0.5); // loud mic untouched
        assert!(meter_level(0.005, 0.01) <= 0.05); // room noise stays low
    }

    #[test]
    fn stopping_without_starting_gives_an_empty_clip() {
        let mic = Microphone::new("");
        let clip = mic.stop();
        assert!(clip.samples.is_empty());
        assert_eq!(clip.sample_rate, 16_000);
    }

    #[test]
    fn takes_at_44100_hz_join_to_the_one_shot_result() {
        let tone: Vec<f32> = (0..44_100 * 3).map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 200.0 / 44_100.0).sin()).collect();
        let whole = to_model_rate(&tone, 44_100);
        let (mut buffer, mut phase, mut pieces) = (Vec::new(), 0.0, Vec::new());
        for chunk in tone.chunks(30_011) {
            buffer.extend_from_slice(chunk);
            pieces.extend(take_resampled(&mut buffer, 44_100, &mut phase));
        }
        pieces.extend(to_model_rate_at(&buffer, 44_100, phase).0); // what `stop` returns
        assert!(whole.len().abs_diff(pieces.len()) <= 2, "{} vs {}", whole.len(), pieces.len());
        assert!(whole.iter().zip(&pieces).all(|(a, b)| (a - b).abs() < 0.05), "pieces drift from the one-shot result");
    }

    #[test]
    fn listing_devices_never_fails() {
        let _ = device_names();
    }
}
