# `crates/oasis-voice/` — microphone and on-device speech-to-text

## Overview

**What:** recording from a microphone, and turning the recording into text with a speech model on
this computer. Nothing is sent anywhere.

**Why a separate crate:** it is the only place that touches the audio device and the speech engine's
native code, so the rest of OASIS sees only the `SttEngine` trait and `AudioClip` (both from
`oasis-models`). Like `oasis-win` and `oasis-dpapi`, it opts out of the workspace `unsafe_code` deny locally
(`#[allow(unsafe_code)]`, D13), with a `SAFETY:` note on each use.

**When:** the first time push-to-talk is used. The speech model takes a few seconds to load, so it
loads on the first recording and stays loaded.

## Interface

**How:**
- `mic.rs`: `Microphone::new(name)`, `start()`, `stop()`, `take()`, `level()`; `device_names()`.
  `stop` and `take` return 16 kHz mono audio (`AudioClip`). `take` returns what was heard since
  `start` or the last `take` while recording goes on, so a long dictation can be cut into pieces
  without a gap. The device is picked by a possibly shortened name (C# saved shortened names), else
  the default. If Windows refuses the device's default format, its other formats are tried (a
  Logitech webcam microphone needed this).
- `resample.rs`: `to_mono` (stereo to mono), `to_model_rate` and `to_model_rate_at` (any rate to
  16 kHz by averaging, so high tones do not fold into speech; `_at` resamples one piece of a longer
  recording and carries the fraction to the next piece), `level` (a level meter), `MODEL_RATE`.
- `sherpa.rs`: sherpa-onnx's C API loaded from `sherpa-onnx-c-api.dll` (D19). The C structs mirror
  `c-api.h` of **2.0.2**; the DLL must be that version. `SherpaTransducer::load` (transducer
  layout) and `load_model` (any `ModelLayout`: transducer, NeMo CTC or Moonshine) give an
  `SttEngine`. `detect_transducer` and `detect_model` find a model's files (int8 preferred on a
  CPU, fp16 on a GPU). `dll_search_folders` looks in `OASIS_SHERPA_DIR`, beside the program, and in
  a `native` folder beside it; `find_dll_folder` picks the first that holds both DLLs.
- `lib.rs`: re-exports `device_names`, `Microphone`, `detect_model`, `detect_transducer`,
  `dll_search_folders`, `find_dll_folder`, `ModelLayout`, `SherpaTransducer`, `TransducerLayout`.
  The `resample` items and `DLL_NAME`, `ONNX_RUNTIME_DLL` are reached through their modules.

**Putting the speech engine in place:** `SW\scripts\copy-speech-engine.ps1 -From <C# OASIS folder>`.

**Checking it by hand:** see [examples/README.md](examples/README.md).

| File | Contents |
|---|---|
| `src/lib.rs` | Module list and re-exports |
| `src/mic.rs` | The microphone |
| `src/resample.rs` | Audio conversion and level |
| `src/sherpa.rs` | The speech engine and where its files are |
| `examples/` | Hand-check and bench tools → [README](examples/README.md) |

## Invariants

- Audio handed to the model is 16 kHz mono (`MODEL_RATE`).
- The sherpa-onnx DLL must match the C structs mirrored from `c-api.h` 2.0.2 (the `lib` folder of the release asset `sherpa-onnx-v2.0.2-win-x64-shared-MT-Release-no-tts`).
- `unsafe` carries a `SAFETY:` note (D13).
- Audio is kept only between `start` and `stop`; before `start` nothing is recorded.
- `SherpaTransducer` is used through `&mut self`, so one thread decodes at a time.

## Side Effects

- Opens the microphone; loads native DLLs at run time and keeps the model in memory; no network.
- `Microphone::new` starts a thread named `oasis-microphone`. The audio stream lives on it (cpal's
  stream is not `Send` everywhere) and the handle talks to it over a channel. `start`, `stop` and
  `take` wait up to 5 seconds for its answer; `stop` and `take` then return empty audio, and
  `start` returns an error. Dropping the `Microphone` stops and joins the thread. `level` reads a
  value the thread keeps up to date.
- Reads env vars `OASIS_SHERPA_DIR` (and, in tests, `OASIS_STT_MODEL_DIR`).

## Verification

- Run: `cargo test -p oasis-voice`. Inline tests cover `mic.rs`, `resample.rs` and `sherpa.rs`.
- With `OASIS_SHERPA_DIR` and `OASIS_STT_MODEL_DIR` set, the test also loads the real model and
  checks the struct layout; without them it skips.
