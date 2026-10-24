# Status

What is built and the decisions behind it, with the measurements that led to them. Last updated 2026-10-24, 11:40.
Everything open is in `NEXT-STEPS.md`.

## Built

Own Tauri 2 app (Rust core, Svelte 5 UI) on sherpa-onnx; not a Meetily fork (decision W0: Meetily is a
meeting-notes product with LLM summaries and its own UX, and Scripta's logic had to be ported anyway).

| Area | State |
|---|---|
| Skeleton | Tauri 2 window with Mica (Windows 11), light/dark/system, tray icon, single instance, logs (14 days) |
| Library | SQLite with migrations; recordings, tracks, transcripts, speakers, folders; search (case- and accent-insensitive) |
| Recording | WASAPI microphone; meetings with system loopback or one app's process tree (e.g. Teams); one QPC timeline, gaps filled with silence; crash-safe WAV (header every second, repaired on launch); pause; switching the microphone while recording; unplugged microphone falls back to the default |
| After recording | microphone levelling (Scripta's algorithm), mix of the meeting tracks, AAC via Media Foundation (96/160 kbit/s) or lossless |
| Import | audio and video via Media Foundation; mp3/m4a/wav/flac copied, other formats (wma, mp4, mov, mkv …) extracted to AAC |
| Transcription | Parakeet TDT 0.6B v3: int8 on the processor, full precision on the graphics card (setting "Grafikkarte verwenden", card automatic or chosen); Silero VAD chunks (≤ 25 s; long audio on up to 4 threads), word timings, Scripta's segmenting; the next track read while one is transcribed; queue, cancel (also while reading), resume after restart, model download with progress, unload after 5 min idle; below normal priority while it runs |
| Speakers | pyannote segmentation 3.0 + TitaNet small in pieces of ≤ 10 min (3 at once), fragments joined by average linkage; Scripta's meeting rules (echo removal, "Ich", microphone diarized only if the room setting is on or the system track is nearly silent); recount with a fixed number |
| Recognition | Scripta's matcher and learning rules incl. all lessons from HANDOVER.md §3 (separate `voices.sqlite`, opt-in) |
| UI | sidebar filters and folders (drag recordings or files onto them), list with search/sort/context menu/swipe to delete, detail with bubbles or text, editing with restore, speaker menus, "Wer spricht hier?" dialog with listening, player (±15 s, speed), export TXT/MD/SRT/VTT/JSON, settings (6 panes), welcome screen with privacy and consent notes, German UI with English translations |
| Distribution | NSIS installer per user (no admin, 18 MB with the DirectML DLLs), release and CI workflows; sherpa-onnx built from source with DirectML (`scripts/build-sherpa-directml.ps1`, cached in CI) |

Verified: 74 unit tests (ports of Scripta's tests plus consolidation, pieces, a wanted number of speakers);
9 end-to-end tests with the real models on the processor and on the RX 7900 XTX (German and English speech,
voices told apart and attributed per sentence, a German meeting with three voices clean and call-like, voices
recognized across sessions); the real 2-hour Teams meeting (6 speakers on the system track, 1 on the
microphone); the installer installs silently and the app starts. UI checked in the browser preview (sample
data) in light and dark.

## Speaker separation on real calls

The owner's 2-hour Teams meeting (`E:\OwnApps\Testaufnahme`, 3–4 people online, only the owner in the room)
first came out with **517 speakers** on the system track and 276 on the microphone track.

- Cause: sherpa-onnx clusters the voice embeddings of its 10-second windows with complete linkage. Over a
  compressed call they are noisy, so each person falls apart into hundreds of parts of a few seconds. A part's
  voice is uncertain: one person's parts were 0.32 alike under 1 s of audio, 0.46 at 1–2 s, 0.60 at 2–4 s,
  0.75 at 16–40 s, 0.84 above 40 s.
- `transcript::consolidate`: with more than 8 parts, those under 2 % of the speech (4–30 s) join the most
  similar speaker, then parts merge by **average linkage** while at least 0.45 alike (one person's fragments
  ≥ 0.45, three different people 0.40–0.44 – a small margin; "Anzahl korrigieren" is the way out). With 8
  parts or fewer sherpa-onnx's split is trusted (fragments under 4 s, voices ≥ 0.8): in the synthetic test
  dialogues two different TTS voices are 0.70 alike, as alike as one real person's parts.
- Tried and dropped: merging by a group's mean voice (chained different people together); fresh voices for
  merged groups and a second pass (a Teams call makes three different people 0.63–0.75 alike).
- Long audio is diarized in pieces of ≤ 10 minutes: one distance matrix over the whole 2-hour track peaked at
  **27 GB** (1.6 GB in pieces). Pieces run on 3 threads at once, each with its own sherpa-onnx instance.
- Recordings over 10 minutes move the segmentation window by 2.5 s instead of 1 s: 104 s instead of 241 s per
  track with the same speakers (5 s merged two main voices; short recordings keep 1 s, the test dialogues lost
  voices with 2.5 s). With 3 pieces at once: 60 s per 2-hour track (sherpa-onnx 48 s, the parts' voices 12 s).
- Calibration tools: `examples/calibrate.rs` (counts as in the app, timings, `SCRIPTA_DUMP` for raw parts).

## Graphics card

- sherpa-onnx ships Windows builds only for the CPU or CUDA. The build script compiles sherpa-onnx 1.13.8
  against onnxruntime-directml 1.24.4 and DirectML 1.15.4 as **static libraries**: Smart App Control blocked a
  self-built `sherpa-onnx-c-api.dll`; only Microsoft-signed DLLs ship (onnxruntime, DirectML, the Visual C++
  runtime 14.44 for PCs without the redistributable).
- The card is chosen by its **LUID** (`SHERPA_ONNX_DML_ADAPTER_LUID`, set with `_putenv_s` because
  `std::env::set_var` doesn't reach the C runtime's `getenv`): adapter numbers differ between sessions and APIs
  (over SSH the RX 7900 XTX was adapter 1, on the desktop 0, and onnxruntime's 0 was the integrated graphics).
- Pitfall: the sherpa-onnx-sys crate copies the static libraries into its compiled rlib and rebuilds only when
  `SHERPA_ONNX_LIB_DIR` changes; the script runs `cargo clean -p sherpa-onnx-sys`, CI keys its cache on it.
- Speech on the card: Parakeet in **full precision** (`ModelKind::SpeechFull`, 2.5 GB; the int8 model was
  slower there and left out sentences: 433 against 491 words in 10 minutes), chunks sorted by length and decoded
  16 at a time (each call costs ≥ ~0.15 s; 10 minutes of speech: 13.4 s one by one, 2.0 s batched; batching
  changes ~3 % of the words). Not batched on the processor (int8 quantizes over the batch).
- Speaker separation stays on the processor (on the card: 14 s against 10 s for 5 minutes).
- The build script keeps Parakeet's decoder and joiner on the processor. On the RX 7900 XTX that made no
  difference (2-hour track, voice detector included: 54.1 s with it, 54.4 and 54.8 s without); it stays for cards
  with a higher cost per call (NVIDIA through DirectML, integrated graphics), where thousands of tiny decoder
  steps would each go to the card.
- A crash in the graphics driver turns the setting off at the next start (`gpu::Guard` marker file).

## Speed (the 2-hour meeting, test PC: Ryzen 5 7600, RX 7900 XTX)

| | 2026-10-23 noon | 2026-10-24 night | now (benchmark) |
|---|---|---|---|
| reading the file, system track | in the next row | ~55 s in the app (129 s minus 74 s) | 14 s |
| voice detector + speech, system track, graphics card | 3.9 min with reading, processor | 74 s | 7 s + 30 s |
| speakers, system track | 4.1 min, 27 GB | 1.0 min, 1.6 GB | 59 s (2 cores left free) |
| whole meeting in the app (room setting on) | 15 min | 4:40 min | **3:17 min** (app, 2026-10-24 11:57) |

- In the app the system track took ~129 s to read and transcribe against ~67 s in the benchmark. Likely cause:
  decoding sent a progress event per AAC frame (~340,000 for 2 hours), each a message to the window's thread
  and a redraw (fixed, see below). The app run afterwards: system track read in 15 s, voice detector 9 s, speech
  31 s, speakers 59 s; the microphone track was read meanwhile (21 s), then 8 s + 21 s + 55 s. The 55 s are the
  room setting (it found 1 speaker): off, the meeting takes about 2:20 min.
- The voice detector runs single-threaded (28 s per 2 hours); long audio is split into up to 4 parts of at least
  5 minutes with a detector each (7 s). Speech across a border comes back as two segments that are joined again;
  the 2-hour track came out with 8237 words instead of 8240.

## Responsiveness

The owner found the app laggy (typing, the cancel button, the progress bar) and the PC unusable while speakers
were separated (CPU at 100 %).

- Progress events: at most every 100 ms and only when the value moved by 0.5 %; the page updates the running
  recording's value in place instead of replacing the map.
- Tauri runs synchronous commands on the window's main thread: all 51 are marked `async` (Tauri's thread pool).
- During a job the process runs at below normal priority (`priority::Background`); Windows applies it to every
  thread, ONNX Runtime's included. The window's thread and the capture threads raise themselves two steps, so
  they stay on par with other programs.
- Work spread over the processor (speaker pieces, voice detectors) leaves 2 logical cores free
  (`engine::FREE_CORES`): the speakers of the 2-hour track took 59 s on 9 threads against 60 s on 12.

## Deviations from HANDOVER.md

- Data in `%LOCALAPPDATA%\io.github.saibot-strg.scripta` instead of `%APPDATA%\Scripta`: local (roaming
  profiles would copy voice data to a server) and apart from the install folder `%LOCALAPPDATA%\Scripta`.
- Speaker embeddings: NVIDIA TitaNet small instead of WeSpeaker ResNet34 (measured, speaker-models.md).
- Parakeet v3 instead of Ultra: Ultra exists in ONNX only from third parties.
- Development loop: the working tree is copied to the PC over SSH (no git pull), so nothing has to be
  committed for a build.

## Development PC notes

- Smart App Control is on: fresh unsigned builds are blocked at random (Code Integrity 3033,
  "Anwendungssteuerungsrichtlinie") – test binaries, tools, even the installer; a rebuild gives a new hash. Never
  turn it off without the owner (it can't be turned on again).
- Over SSH the app can't create its window ("failed to create webview … Ungültiges Fensterhandle"): no desktop
  in that session.
- After `cargo clean -p sherpa-onnx-sys`, Smart App Control once blocked a rebuilt proc-macro DLL
  (`zeroize_derive`); `cargo clean -p zeroize_derive` and a rebuild (new hash) got past it.
- cmd over SSH: `|` inside `powershell -Command "…"` is parsed by cmd (write a `.ps1` and copy it), `set VAR=&&`
  unsets the variable, chain with `&`.
