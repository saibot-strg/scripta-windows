# Status

Last updated 2026-10-23. Keep this file current: what is built, what was verified, what is open.

## Decision W0: own app, not a Meetily fork

The owner wants Scripta mirrored for Windows, not Meetily. Meetily is a meeting-notes product with LLM
summaries and its own UX; adapting it would mean removing more than keeping, and Scripta's logic (library,
speakers, voices, export) had to be ported anyway. Built as an own Tauri 2 app (Rust core, Svelte 5 UI) on
sherpa-onnx.

## Built (milestones W1–W7 at once, as the owner asked)

| Area | State |
|---|---|
| Skeleton | Tauri 2 window with Mica (Windows 11), light/dark/system, tray icon, single instance, logs (14 days) |
| Library | SQLite with migrations; recordings, tracks, transcripts, speakers, folders; search (case- and accent-insensitive) |
| Recording | WASAPI microphone; meetings with system loopback or one app's process tree (e.g. Teams); one QPC timeline, gaps filled with silence; crash-safe WAV (header every second, repaired on launch); pause; switching the microphone while recording; unplugged microphone falls back to the default |
| After recording | microphone levelling (Scripta's algorithm), mix of the meeting tracks, AAC via Media Foundation (96/160 kbit/s) or lossless |
| Import | audio and video via Media Foundation; mp3/m4a/wav/flac copied, other formats (wma, mp4, mov, mkv …) extracted to AAC |
| Transcription | Parakeet TDT 0.6B v3 int8, Silero VAD chunks (≤ 25 s), word timings, Scripta's segmenting; queue, cancel, resume after restart, model download with progress, unload after 5 min idle |
| Speakers | pyannote segmentation 3.0 + TitaNet small, threshold 0.5 (see speaker-models.md); Scripta's meeting rules (echo removal, "Ich", microphone diarized only if the room setting is on or the system track is nearly silent); recount with a fixed number |
| Recognition | Scripta's matcher and learning rules incl. all lessons from HANDOVER.md §3 (separate `voices.sqlite`, opt-in) |
| UI | sidebar filters and folders (drag recordings or files onto them), list with search/sort/context menu/swipe to delete, detail with bubbles or text, editing with restore, speaker menus, "Wer spricht hier?" dialog with listening, player (±15 s, speed), export TXT/MD/SRT/VTT/JSON, settings (6 panes), welcome screen with privacy and consent notes, German UI with English translations |
| Distribution | NSIS installer per user (no admin), release and CI workflows for GitHub Actions |

Not built (later, as in HANDOVER.md): dictation, launch at login.

## Verified

- 68 unit tests (ports of Scripta's tests: segmenting, word assignment, meeting rules, matching, learning
  rules, naming/merging/moving, export, levelling, resampling, timeline, WAV repair, AAC round trip).
- 9 end-to-end tests with the real models: German and English speech, two macOS voices told apart and
  attributed per sentence, fixed speaker count, a German meeting with three Windows voices (clean and
  call-like) separated and attributed per sentence, voices recognized across sessions over a call-like line.
- Performance on the test PC (Ryzen 5 7600, 6 of 12 threads, CPU only): 60 min of audio in 5.2 min
  (transcription 149 s = 24× real time, diarization 163 s = 22×), peak memory 1.2 GB, speech model loads in
  1.2–1.7 s.
- The installer builds (9 MB), installs silently per user and the app starts on the desktop; the welcome
  screen, model download/load and the empty library work (seen in the log and a screenshot).
- UI checked in the browser preview (sample data) in light and dark: list, detail (bubbles/text), recording
  capsule, assignment dialog, settings, welcome, context menus.

## Not verified yet

- **Real recording**: microphone, system loopback, Teams process loopback, timeline alignment, levelling of
  a real headset, pause, microphone switching. Only the code paths with files are tested.
- The 60-minute benchmark found 5 speakers instead of 3 (the same 80 s dialogue repeated 45 times):
  long recordings may be over-split. Calibrate on real meetings; "correct the number" fixes it per recording.
- Office laptops (no strong CPU): expect several times slower than the numbers above.
- Drag and drop from Explorer, the tray menu, the update check against a public release (the repository is
  private, GitHub answers 404).
- Code signing: none yet (docs/DISTRIBUTION.md).

## Speed and what could make it faster

- Everything runs on the CPU with half the logical cores (= the physical cores on the test PC), so the PC
  stays usable. More threads would add only 10–30 % (SMT).
- GPU: sherpa-onnx ships Windows builds only with CUDA (NVIDIA). AMD and Intel GPUs (the owner's RX 7900 XTX,
  office laptops) would need ONNX Runtime with DirectML, i.e. building sherpa-onnx from source with DirectML
  enabled. Intel NPUs would need the OpenVINO execution provider. Both are open.
- 2026-10-23, after the first real use: a single person in an 8 s note came out as 3 speakers, and the
  60-minute benchmark as 5 instead of 3. Diarization now merges speakers with under 4 s of speech into the
  most similar voice and voices at least 0.8 alike (`transcript::consolidate`); an exact number from the
  user is kept. Not yet verified on the PC.

## Deviations from HANDOVER.md

- Data in `%LOCALAPPDATA%\io.github.saibot-strg.scripta` instead of `%APPDATA%\Scripta`: local (roaming
  profiles would copy voice data to a server) and apart from the install folder `%LOCALAPPDATA%\Scripta`.
- Speaker embeddings: NVIDIA TitaNet small instead of WeSpeaker ResNet34 (measured, speaker-models.md).
- Parakeet v3 instead of Ultra: Ultra exists in ONNX only from third parties.
- Development loop: the working tree is copied to the PC over SSH (no git pull), so nothing has to be
  committed for a build.

## Development PC notes

- Smart App Control is on: freshly built unsigned executables are sometimes blocked (Code Integrity 3033);
  the app, the installer and most test runs were allowed. Don't turn it off without the owner.
- A scheduled task `ScriptaAgent` runs `E:\OwnApps\_agent\job.ps1` in the owner's desktop session (UI
  Automation, screenshots). Delete it when no longer needed: `schtasks /delete /tn ScriptaAgent /f`.
