//! Online speech-to-text through Groq's Whisper. Port of `Services/Voice/GroqSpeechClient.cs`.
//!
//! Unlike the engine on this computer, this sends the recording to api.groq.com, so it only runs
//! when the user chose it in the settings (the window asks them to confirm). If the call fails
//! (offline, wrong key, over the rate limit) and "fall back" is on, the same recording is decoded
//! by the local engine instead of being lost.
//!
//! The written forms in the user's Dictionary go with each recording as Whisper's `prompt`, so a
//! rare name ("KiCad") is heard right in the first place (`docs/VOICE_DECISIONS.md` V6). Fixing it
//! afterwards by replacement stays as the second line.

use std::time::Duration;

use oasis_models::{AudioClip, ModelError, SttEngine};

pub const DEFAULT_MODEL: &str = "whisper-large-v3-turbo";
const ENDPOINT: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
/// Groq refuses uploads over 25 MB.
const MAX_UPLOAD_BYTES: usize = 24 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(45);
/// A rate-limit wait longer than this is not worth stalling a paste for.
const MAX_RETRY_WAIT: Duration = Duration::from_millis(2500);
/// Whisper reads at most 224 tokens of prompt; unusual words cost several tokens each, so the
/// list is kept well under that.
const MAX_PROMPT_CHARS: usize = 600;

/// The prompt for a Dictionary of `(spoken, written)` pairs: each written form once, in order,
/// as a comma list, cut at a whole term when it would grow too long. Empty with no Dictionary.
pub fn prompt_for(dictionary: &[(String, String)]) -> String {
    let mut prompt = String::new();
    let mut seen: Vec<String> = Vec::new();
    for (_, written) in dictionary {
        let term = written.split_whitespace().collect::<Vec<_>>().join(" ");
        if term.is_empty() || seen.iter().any(|s| s.eq_ignore_ascii_case(&term)) {
            continue;
        }
        if prompt.len() + term.len() + 2 > MAX_PROMPT_CHARS {
            break;
        }
        if !prompt.is_empty() {
            prompt.push_str(", ");
        }
        prompt.push_str(&term);
        seen.push(term);
    }
    prompt
}

/// The model to ask for: blank means the default, and retired names move to a current one.
pub fn normalize_model(model: &str) -> String {
    match model.trim() {
        "" => DEFAULT_MODEL.to_string(),
        "whisper-large-v3-turbo-preview" | "whisper-1" => DEFAULT_MODEL.to_string(),
        "distil-whisper-large-v3-en" => "whisper-large-v3-turbo".to_string(),
        other => other.to_string(),
    }
}

/// Mono 16-bit PCM in a minimal 44-byte WAV wrapper, which is what Groq is sent.
pub fn wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// The form body Groq expects: the audio file, the model, plain JSON back, and optionally a
/// language and a prompt.
pub fn multipart(boundary: &str, wav: &[u8], model: &str, language: &str, prompt: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav.len() + 600);
    let mut field = |name: &str, value: &str| {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    };
    field("model", model);
    field("response_format", "json");
    field("temperature", "0");
    let language = language.trim();
    if !language.is_empty() && !language.eq_ignore_ascii_case("auto") {
        field("language", language);
    }
    if !prompt.trim().is_empty() {
        field("prompt", prompt.trim());
    }
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"utterance.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// What to tell the user for a failed call.
pub fn describe_error(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str().map(str::to_string)));
    match status {
        401 => "Groq rejected the API key.".into(),
        429 => "Groq rate limit reached; try again shortly.".into(),
        404 => detail.unwrap_or_else(|| "Groq STT model not found.".into()),
        _ => detail.unwrap_or_else(|| format!("Groq STT failed ({status}).")),
    }
}

/// The transcript in Groq's JSON answer.
fn transcript_of(body: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| "Groq returned an unreadable response.".to_string())?;
    Ok(value.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string())
}

enum Failure {
    /// Over the rate limit; Groq said how long to wait, if it did.
    RateLimited(Option<Duration>, String),
    Other(String),
}

/// Sends one recording and returns what Groq heard.
pub fn transcribe(audio: &AudioClip, key: &str, model: &str, language: &str, prompt: &str) -> Result<String, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("The Groq API key is not set. Add one in Settings → Voice.".into());
    }
    if audio.samples.is_empty() {
        return Ok(String::new());
    }
    let wav = wav(&audio.samples, audio.sample_rate);
    if wav.len() > MAX_UPLOAD_BYTES {
        return Err("The recording is too long to upload to Groq.".into());
    }
    let (model, boundary) = (normalize_model(model), format!("oasis-{:x}", wav.len() ^ 0x5bd1_e995));
    let body = multipart(&boundary, &wav, &model, language, prompt);
    // One retry: a burst of dictations can clip the per-minute limit, and Groq says how long to wait.
    match send(&body, &boundary, key) {
        Err(Failure::RateLimited(Some(wait), _)) if wait <= MAX_RETRY_WAIT => {
            std::thread::sleep(wait);
            send(&body, &boundary, key).map_err(|f| match f {
                Failure::RateLimited(_, m) | Failure::Other(m) => m,
            })
        }
        Err(Failure::RateLimited(_, m) | Failure::Other(m)) => Err(m),
        Ok(text) => Ok(text),
    }
}

fn send(body: &[u8], boundary: &str, key: &str) -> Result<String, Failure> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false).build().into();
    let mut response = agent
        .post(ENDPOINT)
        .header("Authorization", format!("Bearer {key}"))
        .content_type(format!("multipart/form-data; boundary={boundary}"))
        .send(body)
        .map_err(|e| Failure::Other(format!("Groq request failed: {e}")))?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(Duration::from_secs_f64);
    let text = response.body_mut().read_to_string().map_err(|e| Failure::Other(format!("Groq request failed: {e}")))?;
    match status {
        200..=299 => transcript_of(&text).map_err(Failure::Other),
        429 => Err(Failure::RateLimited(retry_after, describe_error(status, &text))),
        _ => Err(Failure::Other(describe_error(status, &text))),
    }
}

/// Checks a key without speaking: a quarter second of silence. `None` means it works.
pub fn test_key(key: &str, model: &str) -> Option<String> {
    let silence = AudioClip { samples: vec![0.0; 4000], sample_rate: 16_000 };
    transcribe(&silence, key, model, "en", "").err()
}

/// The online engine, with the local one behind it when the user allowed falling back.
pub struct GroqStt {
    pub key: String,
    pub model: String,
    pub language: String,
    /// [`prompt_for`] the user's Dictionary.
    pub prompt: String,
    pub local: Option<Box<dyn SttEngine>>,
}

impl SttEngine for GroqStt {
    fn transcribe(&mut self, audio: &AudioClip) -> Result<String, ModelError> {
        match transcribe(audio, &self.key, &self.model, &self.language, &self.prompt) {
            Ok(text) => Ok(text),
            Err(reason) => match self.local.as_mut() {
                Some(local) => local.transcribe(audio),
                None => Err(ModelError::Failed(reason)),
            },
        }
    }

    /// Groq gets the Dictionary as its prompt; the names reach the local model kept as a fallback.
    fn set_hotwords(&mut self, phrases: &[String]) {
        if let Some(local) = self.local.as_mut() {
            local.set_hotwords(phrases);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_models_move_to_a_current_one() {
        assert_eq!(normalize_model(""), DEFAULT_MODEL);
        assert_eq!(normalize_model(" whisper-1 "), DEFAULT_MODEL);
        assert_eq!(normalize_model("distil-whisper-large-v3-en"), "whisper-large-v3-turbo");
        assert_eq!(normalize_model("whisper-large-v3"), "whisper-large-v3");
    }

    #[test]
    fn the_wav_header_and_samples() {
        let bytes = wav(&[0.0, 1.0, -1.0, 2.0], 16_000);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 8);
        assert_eq!(bytes.len(), 44 + 8);
        let samples: Vec<i16> = bytes[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(samples, [0, 32767, -32767, 32767], "loud samples are clipped, not wrapped");
    }

    #[test]
    fn the_form_has_each_field_once_and_the_audio_last() {
        let body = multipart("B", b"AUDIO", "m", "en", "KiCad");
        let text = String::from_utf8_lossy(&body);
        for name in ["model", "response_format", "temperature", "language", "prompt", "file"] {
            assert_eq!(text.matches(&format!("name=\"{name}\"")).count(), 1, "{name}");
        }
        assert!(text.ends_with("AUDIO\r\n--B--\r\n"));
        let bare = String::from_utf8_lossy(&multipart("B", b"A", "m", "auto", " ")).to_string();
        assert!(!bare.contains("name=\"language\"") && !bare.contains("name=\"prompt\""));
    }

    #[test]
    fn the_prompt_lists_each_written_form_once_and_stays_short() {
        let pairs = |list: &[(&str, &str)]| list.iter().map(|(s, w)| ((*s).to_string(), (*w).to_string())).collect::<Vec<_>>();
        assert_eq!(prompt_for(&[]), "");
        let dictionary = pairs(&[("kay cad", "KiCad"), ("key cad", "KiCad"), ("oasis", " OASIS  pad "), ("blank", " ")]);
        assert_eq!(prompt_for(&dictionary), "KiCad, OASIS pad");
        let long: Vec<(String, String)> = (0..200).map(|i| (format!("s{i}"), format!("Term{i}"))).collect();
        let prompt = prompt_for(&long);
        assert!(prompt.len() <= MAX_PROMPT_CHARS && prompt.starts_with("Term0, Term1") && !prompt.ends_with(", "), "{prompt}");
    }

    #[test]
    fn errors_are_put_in_plain_words() {
        assert_eq!(describe_error(401, ""), "Groq rejected the API key.");
        assert_eq!(describe_error(429, ""), "Groq rate limit reached; try again shortly.");
        assert_eq!(describe_error(400, r#"{"error":{"message":"bad audio"}}"#), "bad audio");
        assert_eq!(describe_error(500, "not json"), "Groq STT failed (500).");
        assert_eq!(transcript_of(r#"{"text":"  hello "}"#).unwrap(), "hello");
        assert!(transcript_of("<html>").is_err());
    }

    #[test]
    fn no_key_means_no_request_and_a_failure_uses_the_local_engine() {
        let clip = AudioClip { samples: vec![0.1; 100], sample_rate: 16_000 };
        assert!(transcribe(&clip, "  ", "m", "en", "").unwrap_err().contains("API key"));
        struct Local;
        impl SttEngine for Local {
            fn transcribe(&mut self, _: &AudioClip) -> Result<String, ModelError> {
                Ok("local words".into())
            }
        }
        let mut with = GroqStt { key: String::new(), model: String::new(), language: "en".into(), prompt: String::new(), local: Some(Box::new(Local)) };
        assert_eq!(with.transcribe(&clip).unwrap(), "local words");
        let mut without = GroqStt { local: None, ..with };
        assert!(without.transcribe(&clip).is_err());
    }
}
