//! Transcribe every WAV in a folder with one model, one line per clip (`name<TAB>text`), to compare models:
//! `cargo run --release -p oasis-voice --example transcribe_dir -- <wav-folder> <model-folder> <dll-folder> [threads] [greedy|beam:<length_reward>] [max-clips] [hotword,hotword,…]`

use std::path::Path;

use oasis_models::SttEngine;
use oasis_voice::sherpa::Decoding;
use oasis_voice::{detect_model, SherpaTransducer};

#[path = "common/wav.rs"]
mod wav;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [clips, model, dlls, rest @ ..] = args.as_slice() else {
        return Err("usage: transcribe_dir <wav-folder> <model-folder> <dll-folder> [threads]".into());
    };
    let threads = rest.first().and_then(|t| t.parse().ok()).unwrap_or(4);
    let decoding = match rest.get(1).map(String::as_str) {
        Some("greedy") => Decoding::Greedy,
        Some(b) if b.starts_with("beam:") => Decoding::Beam { length_reward: b[5..].parse().map_err(|_| "beam:<number>")? },
        _ => Decoding::default(),
    };
    let max_clips: usize = rest.get(2).and_then(|n| n.parse().ok()).unwrap_or(usize::MAX);
    let layout = detect_model(Path::new(model), false).ok_or("that folder is not a speech model")?;
    let mut engine =
        SherpaTransducer::load_with(Path::new(dlls), &layout, threads, "cpu", decoding).map_err(|e| e.to_string())?;
    if let Some(words) = rest.get(3) {
        engine.set_hotwords(&words.split(',').map(str::to_string).collect::<Vec<_>>());
    }
    let mut names: Vec<_> = std::fs::read_dir(clips)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    names.sort();
    names.truncate(max_clips);
    let (mut audio_s, mut decode_s) = (0.0f64, 0.0f64);
    for path in &names {
        let clip = wav::read_wav(path)?;
        let started = std::time::Instant::now();
        let text = engine.transcribe(&clip).map_err(|e| e.to_string())?;
        decode_s += started.elapsed().as_secs_f64();
        audio_s += clip.samples.len() as f64 / 16_000.0;
        println!("{}\t{}", path.file_stem().unwrap().to_string_lossy(), text.replace('\t', " "));
    }
    eprintln!("{} clips, {audio_s:.0}s audio, decoded in {decode_s:.0}s (x{:.1} real time)", names.len(), audio_s / decode_s);
    Ok(())
}
