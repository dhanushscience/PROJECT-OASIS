//! The three engine traits. Engines are blocking; the host runs them on a dedicated thread (or
//! `spawn_blocking`) and sends the result back as an event, so the reducers stay unchanged.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum ModelError {
    /// Import refused: these required files were not supplied.
    MissingFiles(Vec<String>),
    /// No backend with this id is registered.
    UnknownBackend(String),
    /// The model failed to load or run. The sentence is shown to the user.
    Failed(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::MissingFiles(files) => {
                write!(f, "Missing required files: {}.", files.join(", "))
            }
            ModelError::UnknownBackend(id) => write!(f, "No backend named '{id}'."),
            ModelError::Failed(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for ModelError {}

/// Mono PCM audio.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioClip {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub trait SttEngine: Send {
    fn transcribe(&mut self, audio: &AudioClip) -> Result<String, ModelError>;
    /// Names to listen for in the next recordings (the Dictionary, places, what is on screen).
    /// Engines that cannot be biased ignore them.
    fn set_hotwords(&mut self, _phrases: &[String]) {}
}

pub trait LlmEngine: Send {
    fn complete(&mut self, prompt: &str, max_tokens: usize) -> Result<String, ModelError>;
}

pub trait TtsEngine: Send {
    fn synthesize(&mut self, text: &str) -> Result<AudioClip, ModelError>;
}
