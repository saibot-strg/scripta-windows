# Scripta for Windows

Windows counterpart of Scripta, the local transcription app for the Mac in the sibling folder `OwnTranscriber`.
Same product, same behaviour, built for office PCs.

**Start by reading `docs/HANDOVER.md` completely.** It has the target behaviour, the speaker recognition rules and
lessons learned, the proposed architecture, the development loop and the milestones.

Always:
- `OwnTranscriber` is a read-only reference. Never edit, build, commit or push anything there.
- Everything runs locally; no cloud, no telemetry.
- No personal data in this repository (names, recordings, transcripts, voice profiles); test audio is synthetic.
- Each milestone ends with a gate: build, test, report honestly (including what was not verified), wait for OK.
- UI German with English translations; code, comments and docs in English.
