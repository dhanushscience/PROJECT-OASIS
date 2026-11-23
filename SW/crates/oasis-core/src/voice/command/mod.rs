//! Spoken commands, without the assistant: a transcript becomes one button action by a
//! deterministic router against a closed vocabulary (the C# "Tier A"), and a safety policy decides
//! whether it may run now, must wait for a pad press, or is refused.
//!
//! | Module | C# origin |
//! |---|---|
//! | [`text`] | `VoiceText` in `Services/Voice/VoiceLexicon.cs` |
//! | [`apps`] | `Oasis.Core/Intent/SpokenAppNames.cs`, window aliases, `BrowserOpenIntentParser.cs` |
//! | [`lexicon`] | `Services/Voice/VoiceLexicon.cs` |
//! | [`router`] | `Services/Voice/VoiceCommandRouter.cs` |
//! | [`classifier`] | `Services/Voice/VoiceUtteranceClassifier.cs`: only the "Oasis, …" address is still used |
//! | [`edit`] | New: "copy", "undo", "select line" said on their own press their keys |
//! | [`gate`] | New: is a phrase text to type or a command (`docs/VOICE_DECISIONS.md` V3, V4) |
//! | [`decide`] | New: the whole type-or-run call for one phrase, from facts the engine gathers |
//! | [`intent`] | New: how sure a routed phrase is a command, from the transcript and its target |
//! | [`web_search`] | `Oasis.Core/Intent/WebSearchPhrase.cs`, `WebSearchIntentParser.cs` |
//! | [`reminders`] | alarm, timer, reminder and cancel phrases of `SystemCommandPhrase.cs` |
//! | [`system`] | lock and theme phrases of `Oasis.Core/SystemControl/SystemCommandPhrase.cs` |
//! | [`volume`] | `Features/SystemControl/VolumeIntentParser.cs` (phrases only; keys, not Core Audio) |
//! | [`safety`] | `IntentSafetyPolicy`, `IntentConfirmationPolicy`, `VoiceFastPathGate` |

pub mod apps;
pub mod classifier;
pub mod decide;
pub mod edit;
pub mod gate;
pub mod intent;
pub mod lexicon;
pub mod on_screen;
pub mod reminders;
pub mod router;
pub mod safety;
pub mod spotify;
pub mod system;
pub mod window_cmd;
pub mod text;
pub mod volume;
pub mod web_search;

pub use lexicon::{build_lexicon, LabeledAction, LexiconEntry, LexiconInputs, LexiconKind, LexiconProfile, RunningApp};
pub use router::{route, RouteResult, RouterSettings};
pub use safety::{fast_path, ApprovalMode, FastPath};
