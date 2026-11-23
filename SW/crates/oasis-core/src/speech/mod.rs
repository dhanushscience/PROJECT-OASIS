//! Cleaning up dictated and spoken text. Port of `Oasis.Core/Speech` (C#).
//!
//! | Module | C# origin | What it does |
//! |---|---|---|
//! | [`text`] | `TranscriptText.cs` | Word and token splitting, whitespace and punctuation helpers |
//! | [`numbers`] | `SpokenNumbers*.cs` | "seventy five percent" → "75%", "nineteen ninety nine" → "1999" |
//! | [`punctuation`] | `SpokenPunctuation.cs` | "comma", "full stop", brackets and quotes said aloud become symbols |
//! | [`repair`] | `SpokenRepair.cs` | "Tuesday, no actually Wednesday" keeps only the correction |
//! | [`revision`] | `RevisionRewriter.cs` | "scratch that", "actually 6pm", command restarts |
//! | [`filler`] | `FillerStripper.cs` | Drops "um", "uh" and, for commands, "please", "you know" |
//! | [`repeat`] | `RepeatCollapser.cs` | Collapses the repeats overlapping speech-to-text emits |
//! | [`glossary`] | `GlossaryApplier.cs` | Your own terms and the built-in "Spotify"/"Oasis" homophones |
//! | [`fragments`] | New | A fragment said on its own ("insert my LinkedIn") types its text |
//! | [`formatter`] | `DictationFormatter.cs` | Capital letters and the closing full stop |
//! | [`layout`] | `DictationLayout.cs` | "new line", "new paragraph" and numbered lists |
//! | [`meaning`] | `MeaningPreservation.cs` | Rejects rewrites that invent commands or lose text |
//! | [`pipeline`] | `TranscriptCleanupPipeline.cs` | Runs the stages in order |
//! | [`polish`], [`polish_prompt`] | `TranscriptPolish.cs`, `DictationPolishPrompt.cs` | Optional language-model polish and its safeguards |
//! | [`vocabulary`] | `VocabularyPromotionPolicy.cs` | Learns a spelling after it is corrected twice |
//! | [`terms`] | New | Technical words, units and compounds as written: UART, 3.3 V, 32-bit |
//! | [`style`] | New | How dictation is written for the app in front: chat, mail, notes, exact (code) |
//! | [`audio`] | `NoiseGate.cs`, `EnergyVadPolicy.cs` | Noise gate and silence/valley cut points |

pub mod audio;
pub mod filler;
pub mod fixes;
pub mod formatter;
pub mod fragments;
pub mod glossary;
pub mod hotwords;
pub mod layout;
pub mod meaning;
pub mod numbers;
pub mod pipeline;
pub mod polish;
pub mod polish_prompt;
pub mod punctuation;
pub mod repair;
pub mod repeat;
pub mod revision;
pub mod style;
pub mod terms;
pub mod text;
pub mod vocabulary;

/// What a transcript is for: a command to run, or text to type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupMode {
    Command,
    Dictate,
}

/// A spoken form and what to write instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlossaryEntry {
    pub spoken: String,
    pub canonical: String,
}

impl GlossaryEntry {
    pub fn new(spoken: &str, canonical: &str) -> Self {
        Self { spoken: spoken.to_string(), canonical: canonical.to_string() }
    }
}

/// One change cleanup made: `kind` is "glossary", "revision" or "spelling".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectionPair {
    pub from: String,
    pub to: String,
    pub kind: String,
}

impl CorrectionPair {
    pub fn new(from: &str, to: &str, kind: &str) -> Self {
        Self { from: from.to_string(), to: to.to_string(), kind: kind.to_string() }
    }
}

/// How a transcript is to be cleaned.
#[derive(Debug, Clone)]
pub struct CleanupContext {
    pub mode: CleanupMode,
    /// How the program in front wants dictation written ([`style::for_app`]).
    pub style: style::Style,
    pub glossary: Vec<GlossaryEntry>,
    /// Capital letters, closing full stop and list layout; only for a final transcript.
    pub apply_formatting: bool,
}

impl CleanupContext {
    pub fn command() -> Self {
        Self { mode: CleanupMode::Command, style: style::Style::Default, glossary: Vec::new(), apply_formatting: false }
    }

    pub fn dictate(apply_formatting: bool) -> Self {
        Self { mode: CleanupMode::Dictate, style: style::Style::Default, glossary: Vec::new(), apply_formatting }
    }
}

#[derive(Debug, Clone)]
pub struct CleanupRequest {
    pub raw_text: String,
    pub context: CleanupContext,
}

impl CleanupRequest {
    pub fn new(raw_text: &str, context: CleanupContext) -> Self {
        Self { raw_text: raw_text.to_string(), context }
    }
}

#[derive(Debug, Clone)]
pub struct CleanupResult {
    pub corrected_text: String,
    pub raw_text: String,
    pub corrections: Vec<CorrectionPair>,
    pub used_llm: bool,
}
