# Scripta for Windows – Handover

Scripta is a local transcription app for the Mac (SwiftUI, in the sibling folder `OwnTranscriber`). It records
voice notes and meetings, transcribes them on-device, tells speakers apart and learns who they are. This project
builds **the same product for Windows**, for colleagues in an office. Same name, same behaviour, same rules.
Scripta stays the Mac app; this project does not replace it.

This document is the target behaviour and the rules; keep to it when changing code. What is built and why is
in `STATUS.md`, what is open in `NEXT-STEPS.md`. Use the Scripta source as the reference for behaviour; this
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

Everything above is built except dictation (later, see `NEXT-STEPS.md`).

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
