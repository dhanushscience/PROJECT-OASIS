//! # oasis-voice
//!
//! The microphone and the on-device speech-to-text engine, behind the traits the engine uses.
//! Native code is loaded at run time, so building needs no native toolchain.
//!
//! | Module | Role | C# origin |
//! |---|---|---|
//! | [`mic`] | Record from a named microphone | `Services/Voice/MicrophoneCapture.cs`, `MicrophoneDevices.cs` |
//! | [`resample`] | Mono 16 kHz audio for the model, a level meter | (inside `MicrophoneCapture.cs`) |
//! | [`sherpa`] | sherpa-onnx transducer (Parakeet) speech-to-text | `SpeechRecognizer*.cs`, `SttModelCatalog.cs` |

pub mod cue;
pub mod hotwords;
pub mod mic;
pub mod resample;
pub mod sherpa;

pub use mic::{device_names, Microphone};
pub use sherpa::{
    detect_model, detect_transducer, dll_search_folders, find_dll_folder, ModelLayout,
    SherpaTransducer, TransducerLayout,
};
