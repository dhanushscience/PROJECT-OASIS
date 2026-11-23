//! The on-device speech-to-text engine: sherpa-onnx's C API, loaded from its DLL at run time
//! (DECISIONS D8: sherpa-onnx is what we ship). Port of `SpeechRecognizer.LocalModel.cs` and
//! `SttModelCatalog.cs` for a transducer model such as NVIDIA Parakeet.
//!
//! Loading the DLL when it is needed, instead of linking it, means building OASIS needs no
//! native toolchain, and the app starts (without voice) when the DLL is absent. The structs
//! below mirror `c-api.h` of **sherpa-onnx 2.0.2**, field for field; the DLL must be that
//! version, because the C API passes these structs by pointer and a different layout would be
//! read as garbage. The tests prove the layout with a real model when one is present.
//!
//! A transducer (Parakeet) decodes greedily. A recording with names to listen for (hotwords, given
//! per recording) uses a second recognizer in `modified_beam_search` with `length_reward` 0.5, which
//! 2.0.2 fixed for NeMo TDT models (`docs/OPEN_ANYTHING.md` stage 2, `VOICE_DECISIONS.md` V15).

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;

use libloading::Library;
use oasis_models::{AudioClip, ModelError, SttEngine};

pub const DLL_NAME: &str = "sherpa-onnx-c-api.dll";
/// The DLL's own dependency; it must be in the same folder.
pub const ONNX_RUNTIME_DLL: &str = "onnxruntime.dll";

// ---- the C structs, in header order ------------------------------------------------------------

type Cstr = *const c_char;

#[repr(C)]
struct FeatureConfig {
    sample_rate: i32,
    feature_dim: i32,
}
#[repr(C)]
struct TransducerConfig {
    encoder: Cstr,
    decoder: Cstr,
    joiner: Cstr,
}
#[repr(C)]
struct OneModel {
    model: Cstr,
}
#[repr(C)]
struct WhisperConfig {
    encoder: Cstr,
    decoder: Cstr,
    language: Cstr,
    task: Cstr,
    tail_paddings: i32,
    enable_token_timestamps: i32,
    enable_segment_timestamps: i32,
}
#[repr(C)]
struct SenseVoiceConfig {
    model: Cstr,
    language: Cstr,
    use_itn: i32,
}
#[repr(C)]
struct MoonshineConfig {
    preprocessor: Cstr,
    encoder: Cstr,
    uncached_decoder: Cstr,
    cached_decoder: Cstr,
    merged_decoder: Cstr,
}
#[repr(C)]
struct FireRedAsrConfig {
    encoder: Cstr,
    decoder: Cstr,
}
#[repr(C)]
struct CanaryConfig {
    encoder: Cstr,
    decoder: Cstr,
    src_lang: Cstr,
    tgt_lang: Cstr,
    use_pnc: i32,
}
#[repr(C)]
struct FunAsrNanoConfig {
    encoder_adaptor: Cstr,
    llm: Cstr,
    embedding: Cstr,
    tokenizer: Cstr,
    system_prompt: Cstr,
    user_prompt: Cstr,
    max_new_tokens: i32,
    temperature: f32,
    top_p: f32,
    seed: i32,
    language: Cstr,
    itn: i32,
    hotwords: Cstr,
}
#[repr(C)]
struct Qwen3AsrConfig {
    conv_frontend: Cstr,
    encoder: Cstr,
    decoder: Cstr,
    tokenizer: Cstr,
    max_total_len: i32,
    max_new_tokens: i32,
    temperature: f32,
    top_p: f32,
    seed: i32,
    hotwords: Cstr,
}
#[repr(C)]
struct CohereConfig {
    encoder: Cstr,
    decoder: Cstr,
    language: Cstr,
    use_punct: i32,
    use_itn: i32,
}
#[repr(C)]
struct ModelConfig {
    transducer: TransducerConfig,
    paraformer: OneModel,
    nemo_ctc: OneModel,
    whisper: WhisperConfig,
    tdnn: OneModel,
    tokens: Cstr,
    num_threads: i32,
    debug: i32,
    provider: Cstr,
    model_type: Cstr,
    modeling_unit: Cstr,
    bpe_vocab: Cstr,
    telespeech_ctc: Cstr,
    sense_voice: SenseVoiceConfig,
    moonshine: MoonshineConfig,
    fire_red_asr: FireRedAsrConfig,
    dolphin: OneModel,
    zipformer_ctc: OneModel,
    canary: CanaryConfig,
    wenet_ctc: OneModel,
    omnilingual: OneModel,
    medasr: OneModel,
    funasr_nano: FunAsrNanoConfig,
    fire_red_asr_ctc: OneModel,
    qwen3_asr: Qwen3AsrConfig,
    cohere_transcribe: CohereConfig,
}
#[repr(C)]
struct LmConfig {
    model: Cstr,
    scale: f32,
}
#[repr(C)]
struct HomophoneReplacerConfig {
    dict_dir: Cstr,
    lexicon: Cstr,
    rule_fsts: Cstr,
}
#[repr(C)]
struct CtcFstDecoderConfig {
    graph: Cstr,
    max_active: i32,
}
#[repr(C)]
struct RecognizerConfig {
    feat_config: FeatureConfig,
    model_config: ModelConfig,
    lm_config: LmConfig,
    decoding_method: Cstr,
    max_active_paths: i32,
    hotwords_file: Cstr,
    hotwords_score: f32,
    rule_fsts: Cstr,
    rule_fars: Cstr,
    blank_penalty: f32,
    hr: HomophoneReplacerConfig,
    ctc_fst_decoder_config: CtcFstDecoderConfig,
}
/// Only the first field of the result is read.
#[repr(C)]
struct RecognizerResult {
    text: Cstr,
}

type CreateRecognizer = unsafe extern "C" fn(*const RecognizerConfig) -> *const c_void;
type DestroyRecognizer = unsafe extern "C" fn(*const c_void);
type CreateStream = unsafe extern "C" fn(*const c_void) -> *const c_void;
type CreateStreamWithHotwords = unsafe extern "C" fn(*const c_void, Cstr) -> *const c_void;
type StreamSetOption = unsafe extern "C" fn(*const c_void, Cstr, Cstr);
type DestroyStream = unsafe extern "C" fn(*const c_void);
type AcceptWaveform = unsafe extern "C" fn(*const c_void, i32, *const f32, i32);
type Decode = unsafe extern "C" fn(*const c_void, *const c_void);
type GetResult = unsafe extern "C" fn(*const c_void) -> *const RecognizerResult;
type DestroyResult = unsafe extern "C" fn(*const RecognizerResult);

// ---- which files make a model ------------------------------------------------------------------

/// The files of a transducer model folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransducerLayout {
    pub tokens: PathBuf,
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub joiner: PathBuf,
}

/// Find the model files in `folder`. A folder may hold several builds of the model: int8 is the
/// one for a CPU (quantised ops do not run on the CUDA provider), fp16 for a GPU, and a plain
/// build as the middle choice.
pub fn detect_transducer(folder: &Path, prefer_gpu: bool) -> Option<TransducerLayout> {
    let tokens = folder.join("tokens.txt");
    if !tokens.is_file() {
        return None;
    }
    let builds: [&str; 3] = if prefer_gpu { ["fp16", "", "int8"] } else { ["int8", "", "fp16"] };
    builds.iter().find_map(|build| {
        let suffix = if build.is_empty() { ".onnx".to_string() } else { format!(".{build}.onnx") };
        let (encoder, decoder, joiner) = (
            folder.join(format!("encoder{suffix}")),
            folder.join(format!("decoder{suffix}")),
            folder.join(format!("joiner{suffix}")),
        );
        (encoder.is_file() && decoder.is_file() && joiner.is_file())
            .then(|| TransducerLayout { tokens: tokens.clone(), encoder, decoder, joiner })
    })
}

/// Folders where the native DLLs may be, nearest first: an explicit `OASIS_SHERPA_DIR`, beside the
/// program, and a `native` folder beside it.
pub fn dll_search_folders(exe_dir: Option<&Path>, override_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    folders.extend(override_dir.map(Path::to_path_buf));
    if let Some(dir) = exe_dir {
        folders.push(dir.to_path_buf());
        folders.push(dir.join("native"));
    }
    folders
}

/// The first folder that holds both DLLs.
pub fn find_dll_folder(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|f| f.join(DLL_NAME).is_file() && f.join(ONNX_RUNTIME_DLL).is_file()).cloned()
}

/// Any model this wrapper can load: a transducer (Parakeet TDT), a NeMo CTC model (Parakeet
/// TDT-CTC 110M), or Moonshine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelLayout {
    Transducer(TransducerLayout),
    NemoCtc {
        tokens: PathBuf,
        model: PathBuf,
    },
    Moonshine {
        tokens: PathBuf,
        preprocessor: PathBuf,
        encoder: PathBuf,
        uncached_decoder: PathBuf,
        cached_decoder: PathBuf,
    },
}

/// The model in `folder`, of whichever kind: a transducer first, then a single-file CTC model
/// (`model.int8.onnx` / `model.onnx`), then Moonshine (`preprocess.onnx` and three `.int8.onnx`).
pub fn detect_model(folder: &Path, prefer_gpu: bool) -> Option<ModelLayout> {
    if let Some(t) = detect_transducer(folder, prefer_gpu) {
        return Some(ModelLayout::Transducer(t));
    }
    let tokens = folder.join("tokens.txt");
    if !tokens.is_file() {
        return None;
    }
    if let Some(model) = ["model.int8.onnx", "model.onnx"]
        .iter()
        .map(|n| folder.join(n))
        .find(|p| p.is_file())
    {
        return Some(ModelLayout::NemoCtc { tokens, model });
    }
    let file = |n: &str| Some(folder.join(n)).filter(|p| p.is_file());
    Some(ModelLayout::Moonshine {
        preprocessor: file("preprocess.onnx")?,
        encoder: file("encode.int8.onnx")?,
        uncached_decoder: file("uncached_decode.int8.onnx")?,
        cached_decoder: file("cached_decode.int8.onnx")?,
        tokens,
    })
}

// ---- the engine --------------------------------------------------------------------------------

struct Api {
    // Keeps the DLL loaded for as long as the function pointers are in use.
    _library: Library,
    create_recognizer: CreateRecognizer,
    destroy_recognizer: DestroyRecognizer,
    create_stream: CreateStream,
    destroy_stream: DestroyStream,
    accept_waveform: AcceptWaveform,
    decode: Decode,
    get_result: GetResult,
    destroy_result: DestroyResult,
    create_stream_with_hotwords: CreateStreamWithHotwords,
    stream_set_option: StreamSetOption,
}

/// The extra score for every word piece of a hotword. sherpa-onnx's default; a higher one makes the
/// model hear names that were not said.
const HOTWORDS_SCORE: f32 = 1.5;
/// Added for every non-blank piece in beam search, so a pause does not end the text (sherpa-onnx
/// pull request 4084: 0.5 for Parakeet v2 and v3).
const LENGTH_REWARD: f32 = 0.5;

/// How a transducer turns the model's scores into text. CTC and Moonshine models are always greedy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decoding {
    Greedy,
    /// `modified_beam_search`: hotwords apply; `length_reward` keeps text after a pause.
    Beam { length_reward: f32 },
}

impl Default for Decoding {
    /// Greedy: beam search decodes about 1.7 times slower (20 clips: 19 s against 32 s), which
    /// dictation must not pay. Names are listened for by a second, beam recognizer made on demand.
    fn default() -> Decoding {
        Decoding::Greedy
    }
}

pub struct SherpaTransducer {
    api: Api,
    recognizer: *const c_void,
    /// Made on the first hotwords for a transducer that decodes greedily: the same model again, in
    /// beam search. A recording with hotwords uses it, any other the greedy `recognizer`.
    beam_recognizer: *const c_void,
    /// Whether `recognizer` itself decodes by beam search (hotwords and the length reward apply).
    beam: bool,
    /// What a second recognizer is made from.
    layout: ModelLayout,
    tokens: CString,
    files: Vec<CString>,
    provider: CString,
    num_threads: i32,
    length_reward: CString,
    /// The model's word pieces, to write names as pieces.
    vocab: std::collections::HashSet<String>,
    /// The hotwords text for the next recordings, already as pieces; empty for none.
    hotwords: CString,
}

// SAFETY: the recognizer is only used through `&mut self`, so one thread uses it at a time, and
// sherpa-onnx recognizers have no thread affinity.
#[allow(unsafe_code)]
unsafe impl Send for SherpaTransducer {}

impl SherpaTransducer {
    /// Load the DLL from `dll_folder` and the model from `layout`. `num_threads` is how many CPU
    /// threads decoding may use; `provider` is `"cpu"` (or `"cuda"` with the CUDA build).
    pub fn load(
        dll_folder: &Path,
        layout: &TransducerLayout,
        num_threads: i32,
        provider: &str,
    ) -> Result<SherpaTransducer, ModelError> {
        SherpaTransducer::load_model(
            dll_folder,
            &ModelLayout::Transducer(layout.clone()),
            num_threads,
            provider,
        )
    }

    /// [`SherpaTransducer::load`] for any [`ModelLayout`].
    pub fn load_model(
        dll_folder: &Path,
        layout: &ModelLayout,
        num_threads: i32,
        provider: &str,
    ) -> Result<SherpaTransducer, ModelError> {
        SherpaTransducer::load_with(dll_folder, layout, num_threads, provider, Decoding::default())
    }

    /// [`SherpaTransducer::load_model`] with a chosen [`Decoding`] (the benches compare them).
    #[allow(unsafe_code)]
    pub fn load_with(
        dll_folder: &Path,
        layout: &ModelLayout,
        num_threads: i32,
        provider: &str,
        decoding: Decoding,
    ) -> Result<SherpaTransducer, ModelError> {
        let fail = |why: String| ModelError::Failed(why);
        let dll = dll_folder.join(DLL_NAME);
        // SAFETY: loading a DLL runs its initialisation; this is the sherpa-onnx DLL the user
        // placed next to the program. ALTERED_SEARCH_PATH makes Windows look for onnxruntime.dll
        // in the same folder rather than only in the program's.
        let library = unsafe {
            libloading::os::windows::Library::load_with_flags(&dll, 0x0000_0008)
                .map(Library::from)
                .map_err(|e| fail(format!("The speech engine ({}) could not be loaded: {e}", dll.display())))?
        };
        macro_rules! symbol {
            ($name:literal, $ty:ty) => {
                // SAFETY: the symbol name and type are those of sherpa-onnx's c-api.h.
                *unsafe { library.get::<$ty>(concat!($name, "\0").as_bytes()) }
                    .map_err(|e| fail(format!("The speech engine is missing {}: {e}", $name)))?
            };
        }
        let create_recognizer: CreateRecognizer = symbol!("SherpaOnnxCreateOfflineRecognizer", CreateRecognizer);
        let destroy_recognizer: DestroyRecognizer = symbol!("SherpaOnnxDestroyOfflineRecognizer", DestroyRecognizer);
        let create_stream: CreateStream = symbol!("SherpaOnnxCreateOfflineStream", CreateStream);
        let create_stream_with_hotwords: CreateStreamWithHotwords =
            symbol!("SherpaOnnxCreateOfflineStreamWithHotwords", CreateStreamWithHotwords);
        let stream_set_option: StreamSetOption = symbol!("SherpaOnnxOfflineStreamSetOption", StreamSetOption);
        let destroy_stream: DestroyStream = symbol!("SherpaOnnxDestroyOfflineStream", DestroyStream);
        let accept_waveform: AcceptWaveform = symbol!("SherpaOnnxAcceptWaveformOffline", AcceptWaveform);
        let decode: Decode = symbol!("SherpaOnnxDecodeOfflineStream", Decode);
        let get_result: GetResult = symbol!("SherpaOnnxGetOfflineStreamResult", GetResult);
        let destroy_result: DestroyResult = symbol!("SherpaOnnxDestroyOfflineRecognizerResult", DestroyResult);

        let text = |p: &Path| {
            CString::new(p.to_string_lossy().as_bytes())
                .map_err(|_| fail("A model path has an invalid character.".into()))
        };
        // The model's files, in the order the config below reads them; the strings must outlive the create call.
        let (tokens, files): (CString, Vec<CString>) = match layout {
            ModelLayout::Transducer(t) => (
                text(&t.tokens)?,
                vec![text(&t.encoder)?, text(&t.decoder)?, text(&t.joiner)?],
            ),
            ModelLayout::NemoCtc { tokens, model } => (text(tokens)?, vec![text(model)?]),
            ModelLayout::Moonshine {
                tokens,
                preprocessor,
                encoder,
                uncached_decoder,
                cached_decoder,
            } => (
                text(tokens)?,
                vec![
                    text(preprocessor)?,
                    text(encoder)?,
                    text(uncached_decoder)?,
                    text(cached_decoder)?,
                ],
            ),
        };
        let provider = CString::new(provider).map_err(|_| fail("Invalid provider name.".into()))?;
        // Beam search (and so hotwords) only for a transducer; CTC and Moonshine stay greedy.
        let transducer = matches!(layout, ModelLayout::Transducer(_));
        let (beam, length_reward) = match (transducer, decoding) {
            (true, Decoding::Beam { length_reward }) => (true, length_reward),
            (true, Decoding::Greedy) => (false, LENGTH_REWARD), // for the beam recognizer made later
            _ => (false, 0.0),
        };
        let vocab: std::collections::HashSet<String> = if transducer {
            std::fs::read_to_string(tokens.to_string_lossy().as_ref())
                .unwrap_or_default()
                .lines()
                .filter_map(|l| l.rsplit_once(' ').map(|(piece, _)| piece.to_string()))
                .collect()
        } else {
            Default::default()
        };

        let mut engine = SherpaTransducer {
            api: Api {
                _library: library,
                create_recognizer,
                destroy_recognizer,
                create_stream,
                destroy_stream,
                accept_waveform,
                decode,
                get_result,
                destroy_result,
                create_stream_with_hotwords,
                stream_set_option,
            },
            recognizer: ptr::null(),
            beam_recognizer: ptr::null(),
            beam,
            layout: layout.clone(),
            tokens,
            files,
            provider,
            num_threads,
            length_reward: CString::new(length_reward.to_string()).expect("no NUL"),
            vocab,
            hotwords: CString::default(),
        };
        engine.recognizer = engine.create(beam);
        if engine.recognizer.is_null() {
            return Err(fail("The speech model could not be loaded (its files may be damaged or not a supported model).".into()));
        }
        Ok(engine)
    }

    /// A recognizer of this model, decoding by beam search or greedily. Null when it cannot be made.
    #[allow(unsafe_code)]
    fn create(&self, beam: bool) -> *const c_void {
        let (greedy, beam_search) = (c"greedy_search", c"modified_beam_search");
        // SAFETY: every field of these structs is an integer, a float or a pointer, for which
        // all-zero is the documented "not set" value.
        let mut config: RecognizerConfig = unsafe { std::mem::zeroed() };
        config.feat_config = FeatureConfig {
            sample_rate: 16_000,
            feature_dim: 80,
        };
        let f = |i: usize| self.files[i].as_ptr();
        match &self.layout {
            ModelLayout::Transducer(_) => {
                config.model_config.transducer = TransducerConfig {
                    encoder: f(0),
                    decoder: f(1),
                    joiner: f(2),
                }
            }
            ModelLayout::NemoCtc { .. } => config.model_config.nemo_ctc = OneModel { model: f(0) },
            ModelLayout::Moonshine { .. } => {
                config.model_config.moonshine = MoonshineConfig {
                    preprocessor: f(0),
                    encoder: f(1),
                    uncached_decoder: f(2),
                    cached_decoder: f(3),
                    merged_decoder: ptr::null(),
                };
            }
        }
        config.model_config.tokens = self.tokens.as_ptr();
        config.model_config.num_threads = self.num_threads.max(1);
        config.model_config.provider = self.provider.as_ptr();
        config.decoding_method = if beam { beam_search.as_ptr() } else { greedy.as_ptr() };
        config.max_active_paths = 4;
        config.hotwords_score = HOTWORDS_SCORE;
        // SAFETY: `config` is fully initialised and every string it points to outlives the call.
        unsafe { (self.api.create_recognizer)(&config) }
    }
}

impl SttEngine for SherpaTransducer {
    #[allow(unsafe_code)]
    fn transcribe(&mut self, audio: &AudioClip) -> Result<String, ModelError> {
        if audio.samples.is_empty() {
            return Ok(String::new());
        }
        let count = i32::try_from(audio.samples.len()).map_err(|_| ModelError::Failed("The recording is too long.".into()))?;
        let rate = i32::try_from(audio.sample_rate).map_err(|_| ModelError::Failed("Invalid sample rate.".into()))?;
        // SAFETY: the recognizer is alive (owned by self); the stream is created, used and
        // destroyed here; the sample buffer outlives the calls; the result is read before it is
        // destroyed and its text is copied out.
        unsafe {
            let named = !self.hotwords.is_empty() && !self.beam_recognizer.is_null();
            let recognizer = if named { self.beam_recognizer } else { self.recognizer };
            let beam = self.beam || named;
            let stream = if beam && !self.hotwords.is_empty() {
                (self.api.create_stream_with_hotwords)(recognizer, self.hotwords.as_ptr())
            } else {
                (self.api.create_stream)(recognizer)
            };
            if stream.is_null() {
                return Err(ModelError::Failed("The speech engine could not start a recognition.".into()));
            }
            if beam {
                (self.api.stream_set_option)(stream, c"length_reward".as_ptr(), self.length_reward.as_ptr());
            }
            (self.api.accept_waveform)(stream, rate, audio.samples.as_ptr(), count);
            (self.api.decode)(recognizer, stream);
            let result = (self.api.get_result)(stream);
            let text = if result.is_null() || (*result).text.is_null() {
                String::new()
            } else {
                CStr::from_ptr((*result).text).to_string_lossy().trim().to_string()
            };
            if !result.is_null() {
                (self.api.destroy_result)(result);
            }
            (self.api.destroy_stream)(stream);
            Ok(text)
        }
    }

    fn set_hotwords(&mut self, phrases: &[String]) {
        if self.vocab.is_empty() {
            return; // not a transducer
        }
        let text = crate::hotwords::lines(phrases, &self.vocab);
        self.hotwords = CString::new(text).unwrap_or_default();
        // A one-off cost (the model loads a second time) for whoever speaks commands; dictation never pays it.
        if !self.beam && self.beam_recognizer.is_null() && !self.hotwords.is_empty() {
            self.beam_recognizer = self.create(true);
        }
    }
}

impl Drop for SherpaTransducer {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        if !self.recognizer.is_null() {
            // SAFETY: created by `create_recognizer`, destroyed once, before the DLL unloads
            // (the library field drops after this).
            unsafe { (self.api.destroy_recognizer)(self.recognizer) };
            self.recognizer = ptr::null();
        }
        if !self.beam_recognizer.is_null() {
            // SAFETY: as above.
            unsafe { (self.api.destroy_recognizer)(self.beam_recognizer) };
            self.beam_recognizer = ptr::null();
        }
        let _ = self.api.create_recognizer; // the pointers die with the library
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oasis-sherpa-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(dir: &Path, names: &[&str]) {
        for n in names {
            fs::write(dir.join(n), "x").unwrap();
        }
    }

    #[test]
    fn a_transducer_folder_is_recognised_and_int8_is_preferred_on_a_cpu() {
        let dir = scratch("int8");
        touch(&dir, &["tokens.txt", "encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "encoder.fp16.onnx", "decoder.fp16.onnx", "joiner.fp16.onnx"]);
        let cpu = detect_transducer(&dir, false).unwrap();
        assert!(cpu.encoder.ends_with("encoder.int8.onnx"));
        assert!(detect_transducer(&dir, true).unwrap().encoder.ends_with("encoder.fp16.onnx"));
    }

    #[test]
    fn a_plain_build_is_used_when_there_is_no_other() {
        let dir = scratch("plain");
        touch(&dir, &["tokens.txt", "encoder.onnx", "decoder.onnx", "joiner.onnx"]);
        assert!(detect_transducer(&dir, false).unwrap().joiner.ends_with("joiner.onnx"));
    }

    #[test]
    fn an_incomplete_folder_is_not_a_model() {
        let dir = scratch("partial");
        touch(&dir, &["tokens.txt", "encoder.int8.onnx", "decoder.int8.onnx"]);
        assert_eq!(detect_transducer(&dir, false), None, "no joiner");
        let no_tokens = scratch("notokens");
        touch(&no_tokens, &["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx"]);
        assert_eq!(detect_transducer(&no_tokens, false), None);
        assert_eq!(detect_transducer(Path::new(r"C:\no\such\folder"), false), None);
    }

    #[test]
    fn a_ctc_or_moonshine_folder_is_recognised_and_a_transducer_wins() {
        let ctc = scratch("ctc");
        touch(&ctc, &["tokens.txt", "model.int8.onnx"]);
        assert!(
            matches!(detect_model(&ctc, false), Some(ModelLayout::NemoCtc { model, .. }) if model.ends_with("model.int8.onnx"))
        );
        let moon = scratch("moon");
        touch(
            &moon,
            &[
                "tokens.txt",
                "preprocess.onnx",
                "encode.int8.onnx",
                "uncached_decode.int8.onnx",
                "cached_decode.int8.onnx",
            ],
        );
        assert!(matches!(
            detect_model(&moon, false),
            Some(ModelLayout::Moonshine { .. })
        ));
        touch(
            &moon,
            &["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx"],
        );
        assert!(matches!(
            detect_model(&moon, false),
            Some(ModelLayout::Transducer(_))
        ));
        let half = scratch("halfmoon");
        touch(
            &half,
            &["tokens.txt", "preprocess.onnx", "encode.int8.onnx"],
        );
        assert_eq!(detect_model(&half, false), None);
    }

    #[test]
    fn the_dll_folder_needs_both_files() {
        let (both, one) = (scratch("both"), scratch("one"));
        touch(&both, &[DLL_NAME, ONNX_RUNTIME_DLL]);
        touch(&one, &[DLL_NAME]);
        assert_eq!(find_dll_folder(&[one.clone(), both.clone()]), Some(both));
        assert_eq!(find_dll_folder(&[one]), None);
    }

    #[test]
    fn the_search_order_is_override_then_beside_the_program() {
        let found = dll_search_folders(Some(Path::new(r"C:\app")), Some(Path::new(r"C:\mine")));
        assert_eq!(found, [PathBuf::from(r"C:\mine"), PathBuf::from(r"C:\app"), PathBuf::from(r"C:\app\native")]);
    }

    #[test]
    fn a_missing_dll_is_a_sentence_not_a_crash() {
        let dir = scratch("nodll");
        let layout = TransducerLayout { tokens: dir.join("t"), encoder: dir.join("e"), decoder: dir.join("d"), joiner: dir.join("j") };
        let error = SherpaTransducer::load(&dir, &layout, 2, "cpu").err().expect("fails");
        assert!(error.to_string().contains("could not be loaded"), "{error}");
    }

    /// With a real model and the DLLs (set OASIS_SHERPA_DIR and OASIS_STT_MODEL_DIR), the engine
    /// loads and decodes. Proves the struct layout above against the real DLL.
    #[test]
    fn a_real_model_loads_and_silence_gives_no_words() {
        let (Some(dlls), Some(model)) = (std::env::var_os("OASIS_SHERPA_DIR"), std::env::var_os("OASIS_STT_MODEL_DIR")) else {
            eprintln!("skipped: OASIS_SHERPA_DIR and OASIS_STT_MODEL_DIR are not set");
            return;
        };
        let layout = ModelLayout::Transducer(detect_transducer(Path::new(&model), false).expect("a model folder"));
        let silence = AudioClip { samples: vec![0.0; 16_000], sample_rate: 16_000 };
        let mut greedy = SherpaTransducer::load_with(Path::new(&dlls), &layout, 2, "cpu", Decoding::Greedy).expect("the engine loads");
        assert_eq!(greedy.transcribe(&silence).unwrap(), "");
        // Beam search with the length reward may answer pure silence with one filler ("Mm-hmm.",
        // "Yeah."; 2.0.2, Parakeet v2), which is why silent audio never reaches it (voice.rs
        // `has_speech`). Never more than that, and with hotwords set it still decodes.
        let mut beam = SherpaTransducer::load_model(Path::new(&dlls), &layout, 2, "cpu").expect("the engine loads");
        beam.set_hotwords(&["recycle bin".to_string(), "OASIS".to_string()]);
        let heard = beam.transcribe(&silence).unwrap();
        assert!(heard.split_whitespace().count() <= 1, "{heard}");
    }
}
