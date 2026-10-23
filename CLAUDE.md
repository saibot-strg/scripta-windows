# Scripta for Windows

Windows counterpart of Scripta, the local transcription app for the Mac in the sibling folder `OwnTranscriber`.
Same product, same behaviour, built for office PCs.

**Read `docs/HANDOVER.md` (target behaviour, rules, lessons) and `docs/STATUS.md` (what is built, decisions,
what is open) before changing anything.**

Always:

- `OwnTranscriber` is a read-only reference. Never edit, build, commit or push anything there.
- Everything runs locally; no cloud, no telemetry.
- No personal data in this repository (names, recordings, transcripts, voice profiles); test audio is synthetic.
- Each milestone ends with a gate: build, test, report honestly (including what was not verified), wait for OK.
- UI German with English translations (`src/lib/i18n.en.ts`, keyed by the German text); code, comments and
  docs in English.

Stack: Tauri 2 (Rust in `src-tauri/`, Svelte 5 UI in `src/`), sherpa-onnx (Parakeet, pyannote, TitaNet),
WASAPI capture (`wasapi` crate), Media Foundation for decoding and AAC, SQLite (`rusqlite`).

Development loop (agent on the Mac, test PC over SSH via Tailscale, see `docs/STATUS.md`):

- Copy the working tree to the PC (no git round trip): `tar` over `ssh pc` into `E:\OwnApps\scripta-windows`.
- Build and test there: `cargo test --lib`, `cargo test --release --test end_to_end -- --test-threads=1`,
  `npx tauri build --target x86_64-pc-windows-msvc` (the explicit target is required, see `.cargo/config.toml`).
- UI without the PC: `npm run dev` and open <http://localhost:1420> (sample data from `src/mock.ts`).
- GUI and audio can't be tested over SSH; the owner tries them on the PC. Logs: `%LOCALAPPDATA%\io.github.saibot-strg.scripta\Logs`.
