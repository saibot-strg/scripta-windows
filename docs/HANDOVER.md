# Scripta for Windows – Handover

Scripta is a local transcription app for the Mac (SwiftUI, in the sibling folder `OwnTranscriber`). It records
voice notes and meetings, transcribes them on-device, tells speakers apart and learns who they are. This project
builds **the same product for Windows**, for colleagues in an office. Same name, same behaviour, same rules.
Scripta stays the Mac app; this project does not replace it.

Read this document fully before writing code. Use the Scripta source as the reference for behaviour; this
document says where to look.

## 1. Rules

- **`OwnTranscriber` is read-only.** Read it, never edit, build, commit or push in it. All work happens in
  `scripta-windows`.
- **Everything runs locally.** Audio, transcripts and voice profiles never leave the PC. No cloud APIs, no
  telemetry. Models are downloaded once (Hugging Face), then the app works offline.
- **No personal data in the repository**: no real names, recordings, transcripts or voice profiles of anyone,
  not in code, tests, fixtures, docs or commit messages. Test audio is synthetic (TTS voices) only.
- **Not in scope, decided by the owner:** a people sidebar with photos, and uploading audio of people to train
  their voice. Both are out for privacy reasons; don't propose them again.
- **Milestones end with a gate.** After each milestone: build, run the tests, summarize what works and what was
  not verified, then wait for the owner's OK. Report honestly; say what you could not test.
- **Commits:** small and descriptive, in English. The owner decides about releases and tags.
- **Languages:** UI German (source language) with English translations. Code comments, README and docs English.

## 2. What Scripta does (the target behaviour)

| Area | Behaviour | Reference in Scripta |
|------|-----------|----------------------|
| Recording | Microphone recordings; meetings with two tracks: microphone ("Ich") and system audio ("Andere"), time-aligned, plus a mix for playback | `Scripta/AudioCapture/` |
| Import | Audio and video files via menu or drag and drop | `Scripta/AudioImport/` |
| Transcription | NVIDIA Parakeet (multilingual, automatic language), word timings, segments at sentence ends / 1.2 s pauses / 30 s max | `Scripta/Transcription/ParakeetEngine.swift`, `TranscriptBuilder.swift` |
| Speaker separation | Diarization per track; meetings: system-track voices become "Sprecher 1, 2 …", the main microphone voice stays "Ich" | `TranscriptBuilder.swift`, `SpeakerDiarizer.swift`, `TranscriptionService.swift` |
| Voice recognition | Opt-in; people with voice samples; guesses marked "?"; automatic names after confirmations | `Scripta/Voices/` |
| Assigning speakers | Dialog "Wer spricht hier?", per-bubble reassignment, merge, correct the number of speakers | `Scripta/UI/Detail/SpeakerAssignment.swift`, `SegmentEditing.swift` |
| Library | Sidebar filters (source, status), folders with drag and drop, search in titles and transcripts, swipe to delete | `Scripta/UI/Sidebar/`, `Scripta/UI/Library/`, `Scripta/Library/` |
| Transcript view | Chat bubbles or text; timestamp plus clock time; click to play from there; edit text | `Scripta/UI/Detail/` |
| Export | TXT, Markdown, SRT, WebVTT, JSON (speaker names included) | `Scripta/Export/TranscriptExporter.swift` |
| Dictation | Global hotkey, speak, text is pasted into the active app | `Scripta/Dictation/` |
| Updates | Checks GitHub releases once a day, shows a note | `Scripta/Updates/` |

Priority for Windows: recording + meetings, transcription, speaker separation and recognition, library, export.
Dictation and the update check come later.

## 3. Speaker separation and recognition – the details that matter

This is the part that took the most iterations on the Mac. Copy the logic, not just the idea.

### Pipeline per recording
1. Transcribe each track (words with timings).
2. Diarize the track: speaker turns plus one voice embedding (fingerprint) per speaker.
3. Assign every word to the turn it overlaps most (else the nearest turn); split segments at speaker changes.
4. Meetings: system-track speakers are numbered; the microphone is **only diarized** if the setting "Mehrere
   Personen am Mikrofon" is on or the system track is nearly silent (< 20 words, an in-person meeting recorded in
   meeting mode). In an online meeting only the user speaks into the microphone.
5. If the microphone is diarized: the voice with most speech after echo removal is "Ich"; further voices with
   less than 8 s of speech are folded into "Ich" (echo, split-offs); others become numbered speakers.
6. Renumber speakers by first appearance; carry each speaker's embedding along.
7. If diarization fails, keep the transcript with "Ich"/"Andere". Diarization is a bonus, never a failure.

### Recognition (opt-in, "Stimmen wiedererkennen", off by default)
- Store per transcript speaker its anonymous embedding (deleted with the recording). People and their samples
  exist only after opt-in.
- Score of a speaker against a person = mean of the 3 highest cosine similarities to that person's samples.
- Each person goes to at most one speaker per transcript (greedy by score).
- Thresholds (Scripta values, calibrate on real data):
  - below 0.5 → unknown ("Sprecher 2")
  - ≥ 0.5 → **guess**: the name is filled in, marked with "?", teaches nothing until confirmed
  - automatic (no "?") only if score ≥ 0.7 **and** margin to the second-best person ≥ 0.1 **and** the person has
    ≥ 3 confirmed samples **and** the speaker talked ≥ 10 s
  - no match at all under 3 s of speech; no sample under 5 s
- Learning: naming or confirming a speaker adds its embedding as a confirmed sample. At most 40 samples per
  person; a sample ≥ 0.995 similar to an existing one replaces it (same recording transcribed again).

### Lessons learned (bugs we had – do not repeat)
1. **Apply names from the assignment dialog together.** Applying them one by one merged speakers when two guessed
   names were swapped (A↔B): the first rename collided with the other speaker's old name. Compute the final names
   first; merge only speakers whose *final* names are equal.
2. **Merging teaches only if the voice sounds like the person** (score ≥ 0.5 against the person's samples).
   Users merge to clean up mix-ups; a dissimilar voice would poison the profile.
3. **One voice belongs to one person.** When a voice is assigned to person B, remove the same voice (same speaker
   or ≥ 0.995 similar) from every other person.
4. **A speaker with passages moved out of it is mixed:** drop its embedding and any sample it gave.
5. **Never learn from guesses.** Only confirmed names create confirmed samples.
6. A `ModelContainer`-like owner must outlive the context that uses it (Scripta crashed when the container was
   released) – applies to whatever storage you use.

### Expectations to state to users (onboarding, docs)
- **Names improve with use:** each person is named once by hand, then guessed ("?"), after about three confirmed
  meetings recognized with confidence.
- **Voice separation does not learn by itself.** It depends on the model and the audio (similar voices, a shared
  microphone, compressed calls). Mistakes are corrected by hand: correct the number of speakers, merge, move
  single bubbles.
- Measured on the Mac: one remote person was sometimes split into 3 speakers; in a 3-person call both available
  models agreed on the voices. "Correct the number of speakers" re-runs diarization with an exact count.

## 4. Assigning speakers (UX to reproduce)

- Banner above a transcript: "N Stimmen ohne Namen" / "N Namen vermutet" → "Zuordnen …".
- Dialog "Wer spricht hier?": per speaker a colour dot, name field with completion of known people while typing,
  a menu "Aus früheren Meetings" (known people not yet in this meeting) and "Zusammenführen mit" (other speakers,
  each name once), share of speaking time, a sample sentence, ▶ to listen (each click plays another passage,
  3–8 s), "Stimme merken" (off = guest), an opt-in box if recognition is off, and "Anzahl der anderen
  korrigieren" (re-transcribe with an exact number of voices; warns that edits are lost).
- Click on a speaker name above a bubble (or a person button shown on hover): "Diese Sprechblase gehört zu" –
  other speakers, known people, new speaker; changes only that bubble. Plus confirm the guess and "Alle von …
  benennen".
- A "?" next to guessed names; click confirms.
- Settings tab "Personen": opt-in toggle with a privacy note, list of people (confirmed samples, "wird vorgeschlagen"
  / "wird automatisch erkannt"), rename (renames them in all transcripts), delete, "Alle Stimmprofile löschen".

## 5. Proposed architecture (decide after the spike)

- **Stack:** Tauri 2 (Rust core, web UI). Windows only as a target; no need to support macOS.
- **Speech:** sherpa-onnx (Rust/C API, offline, Windows x64/arm64):
  - Parakeet TDT 0.6B v3 int8 ONNX (multilingual). Scripta uses "Parakeet Ultra" (a post-training of v3, CoreML);
    check whether an ONNX export of Ultra exists, else use v3.
  - Diarization: pyannote segmentation 3.0 + a speaker embedding model (WeSpeaker ResNet34 or 3D-Speaker CAM++) +
    clustering. Expose per-speaker embeddings; matching runs on our side (section 3).
- **Audio:** WASAPI. Microphone capture; system audio via process loopback
  (`AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS`, Windows 10 2004+ / Windows 11) for one app (e.g. Teams, include its
  process tree) or the default device loopback for all system audio. Keep both tracks on one timeline (fill gaps
  with silence), as Scripta's `AudioFileSink` does.
- **Storage:** SQLite (e.g. rusqlite) in `%APPDATA%\Scripta`, versioned migrations from day one. Voice data in a
  separate database file so it can be deleted completely.
- **Performance:** expect office laptops without NPU/GPU acceleration. Measure early (section 7). Options if too
  slow or hot: transcribe when plugged in, queue in the background, smaller models.

### Spike first (1–2 days)
Before building, evaluate forking **Meetily** (github.com/Zackriya-Solutions/meetily, MIT, Tauri + Rust +
Next.js, Windows + macOS, Parakeet via ONNX, microphone + system audio). Answer:
1. Does it build on the Windows test PC? How long, which dependencies?
2. Does it capture Teams audio and the microphone cleanly?
3. Real-time factor of Parakeet on the test PC, CPU load, heat, memory for a 60-minute file.
4. Code quality: can our speaker logic and library fit in cleanly?
Recommend fork or own Tauri app with numbers. Keep Meetily's MIT notice if forking.

## 6. Development loop (owner's setup)

- The agent works on the Mac in a workspace with `OwnTranscriber` (read-only) and `scripta-windows`.
- **Fast loop via Tailscale:** the Windows test PC runs the Windows OpenSSH server and is reachable in the owner's
  Tailscale network. Push to the private repo, then build on the PC over SSH, e.g.
  `ssh <pc> "cd C:\dev\scripta-windows; git pull; cargo tauri build"` (shell: PowerShell). Read the errors, fix,
  repeat. Ask the owner for the host name, user and path; never store credentials in the repo.
- GUI and audio cannot be tested over SSH (other session). The owner starts the app on the PC and reports back.
  Make that easy: log to a file in `%APPDATA%\Scripta\logs`, and say exactly what to try.
- **Releases via GitHub Actions** only when a version works: a tag `v*` builds the installer on `windows-latest`
  and attaches it to a GitHub release (like Scripta's `.github/workflows/release.yml`). A manual run builds a test
  artifact only. Unsigned at first (SmartScreen warns – document "More info → Run anyway"); code signing or Intune
  distribution later.

### Windows test PC checklist (for the owner)
Tailscale; OpenSSH Server (Settings → Optional features) with key login; Git; Rust (rustup, MSVC toolchain);
Visual Studio 2022 Build Tools with "Desktop development with C++"; Node.js LTS; WebView2 (built into Windows 11).
The spike may add CMake/LLVM or others – list them in the README.

## 7. Milestones (each ends with a gate)

| # | Milestone | Done when |
|---|-----------|-----------|
| W0 | Spike and decision | Numbers from section 5 on the test PC, recommendation, owner decides |
| W1 | Skeleton | App starts on the PC; library database with migrations; settings; logs; release workflow builds an installer |
| W2 | Recording | Microphone and meeting recording (Teams via process loopback), two aligned tracks + mix, crash-safe |
| W3 | Transcription | Parakeet offline, model download with progress, segments, playback with highlight, search |
| W4 | Speakers | Diarization, numbered speakers, microphone rule, assignment dialog, per-bubble reassignment, recount |
| W5 | Recognition | Voice store, matching, guesses, learning rules and all lessons from section 3, People settings |
| W6 | Library and export | Folders, filters, swipe/delete, TXT/MD/SRT/VTT/JSON |
| W7 | Office readiness | Performance on a weak laptop, installer, onboarding text with expectations and privacy notes |

Tests: unit tests for word assignment, segmenting, microphone rule, matching thresholds, learning rules and the
lessons in section 3 (Scripta's `ScriptaTests/` has the cases: `TranscriptionTests.swift`,
`VoiceRecognitionTests.swift`). End-to-end tests with synthetic TTS dialogues (two clearly different voices).

## 8. Before office rollout (not code, but blocking)

Voice profiles used to identify people are biometric data (Art. 9 GDPR). Before colleagues use recognition:
data protection officer, works council (co-determination for systems suited to monitor employees), a data
protection impact assessment, and consent of everyone recorded (recording without consent is illegal in Germany).
The app must make this easy: recognition opt-in, a clear note, delete per person and all at once, nothing leaves
the PC.
