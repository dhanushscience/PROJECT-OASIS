//! # oasis-engine
//!
//! The running OASIS, without a window. One thread reads the pad, applies the gesture rules,
//! picks the profile for the app in front and hands the button's action to a worker thread.
//! The desktop app and the CLI both start an [`Engine`] and show its [`EngineEvent`]s.
//!
//! Everything that touches the world is passed in ([`EngineParts`]): the pad link, the foreground
//! probe and the action backend. On Windows [`defaults::windows`] supplies the real ones; tests
//! supply fakes and run the whole chain (pad bytes → matched profile → action) with no pad and no
//! desktop.
//!
//! | Module | Role |
//! |---|---|
//! | [`events`] | `EngineEvent`, `Press`, `EngineStatus` |
//! | [`engine`] | `start`, `EngineHandle`, the loop |
//! | `inbox` | What wakes the loop; the pad thread and the front thread that keep slow reads off it |
//! | [`defaults`] | The real Windows parts |

pub mod activity;
pub mod defaults;
pub mod engine;
pub mod events;
pub mod groq;
pub mod history;
mod inbox;
pub mod meetings;
pub mod polish;
pub mod model_download;
pub mod quick_launch;
pub mod reminders;
pub mod self_check;
pub mod spotify;
pub mod stats;
pub mod telemetry;
pub mod use_counts;
pub mod voice;

pub use engine::{
    start, BackendContext, BackendFactory, BridgeConfig, ChangeError, EngineConfig, EngineError,
    EngineHandle, EngineParts, ForegroundProbe,
};
pub use events::{EngineEvent, EngineStatus, FrontApp, Press};
pub use voice::{Heard, VoiceMic, VoiceParts};
