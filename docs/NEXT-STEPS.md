# Next steps

Where to pick up. Last updated 2026-10-24, 13:50. Everything open lives here; `STATUS.md` describes what is
built and why, `HANDOVER.md` the target behaviour and rules.

## Where we stand

- **v0.1.0 is released** (2026-10-24): [GitHub release](https://github.com/saibot-strg/scripta-windows/releases/tag/v0.1.0),
  built by the release workflow from commit "Add graphics card transcription and fix speakers on real calls".
  The repository is public since then (history rewritten before, so all commits carry the noreply address).
- The CI ran green with the DirectML build (53 min the first time: sherpa-onnx 12 min, 74 unit tests, 9
  end-to-end tests on the processor; the runner has Visual Studio 2026). The release run took 27 min.
- Uncommitted: `release.yml` no longer marks 0.x releases as pre-releases (v0.1.0 was one), and this file.
  The owner commits; the agent never commits or branches.
- The v0.1.0 plan agreed on 2026-10-24 is built (details and measurements in `STATUS.md`):
  1. progress events throttled (decoding sent one per audio frame, ~340,000 per 2-hour track),
  2. all commands off the window's main thread,
  3. transcription below normal priority, window and capture threads raised, 2 logical cores left free,
  4. decoder patch measured: no difference on the RX 7900 XTX (54.1 s against 54.4/54.8 s), kept,
  5. reading the file 14 s and the voice detector 28 s per 2-hour track; the detector now runs on up to
     4 threads (7 s), the next track is read while the current one is transcribed, reading can be cancelled,
  6. clearer hint at "Mehrere Personen am Mikrofon".
- Unit tests (74) and end-to-end tests (9, processor and RX 7900 XTX) green. Benchmark of the 2-hour system
  track on the graphics card: reading 14 s, speech 37 s (was 54 s), speakers 59 s.

## The owner's test in the app (2026-10-24 11:57)

- The 2-hour meeting in **3:17 min** (was 4:40), room setting still on (55 s of it for the microphone track,
  1 speaker found); without it about 2:20 min.
- The window stayed responsive, cancelling worked at once.
- CPU at ~83 % during speaker separation: 10 of 12 threads, by design (2 left free). The graphics card peaks at
  ~20 %: it works ~50 of the 197 s; speaker separation (114 s) runs on the processor.
- If the CPU load should drop further: ONNX Runtime's threads busy-wait between tasks
  (`session.intra_op.allow_spinning`, on by default); turning it off needs one more line in the `session.cc`
  patch of the build script.

## Next: the colleague's test (Ryzen 9 9950X, RTX 5080)

- **Turn v0.1.0 into a normal release** (GitHub: Edit release → untick "Set as a pre-release", tick "Set as the
  latest release"). The app's update check asks for `releases/latest`, which leaves pre-releases out; as a
  pre-release no installed copy would ever be told about a newer version.
- **NVIDIA was never tested.** DirectML should drive the RTX 5080, but it's unproven. Ask him to turn on
  "Grafikkarte verwenden", transcribe something long, watch Task Manager (the card's work shows as "3D"), and
  send the log (Settings → Datenschutz → Protokolle). The log line `model loaded model=SpeechFull device=Gpu(…)`
  and `speech recognition device card=…` show which card was used.
- He will see SmartScreen ("Weitere Informationen → Trotzdem ausführen"); with Smart App Control on, a build
  may be blocked outright (it happened four times on the test PC, at random per build).
- Then: read his log; if the RTX 5080 works, the next release is for more colleagues (signing first,
  `DISTRIBUTION.md`).

## Not verified yet

- The app's own flows on the PC beyond transcribing: the graphics card setting's model download (the full model
  was copied by hand on the test PC), the card picker, the crash marker (`gpu::Guard`).
- Real recording: microphone, system loopback, Teams process loopback, timeline alignment, levelling of a real
  headset, pause, microphone switching, recording while a transcription runs (capture threads now raise their
  priority). Only the code paths with files are tested.
- Speaker counts on other real meetings (calibrated on one; the number of small ~1 % groups varies, 6–8).
- The RX 9070 XT, Intel integrated graphics, a real office laptop (a 4-core simulation suggests very roughly
  15–20 min for a 2-hour meeting without a graphics card, measured before the parallel voice detector).
- Drag and drop from Explorer, the tray menu, the update check against a real newer release.

## Later (not v0.1.0)

- Office laptops: several speech chunks at once on the processor; thread counts that follow the process's CPU
  affinity (`available_parallelism` ignores it); an Intel laptop's integrated graphics or NPU (DirectML, or
  OpenVINO); speaker separation while the graphics card transcribes the next track.
- Own speaker separation with batched voice embeddings on the graphics card (sherpa-onnx calls the model
  thousands of times one by one, which made the card slower than the processor).
- If the small speaker groups bother: raise `transcript::FRAGMENT_LIMIT`.
- Dictation and launch at login (HANDOVER.md).
- `actions/cache@v4` still targets Node.js 20 (GitHub warns in every run); move to a newer major version.
- Code signing for the office (`DISTRIBUTION.md`); without it Smart App Control blocks builds at random.
- **Before colleagues use voice recognition** (not code, but blocking): voice profiles are biometric data
  (Art. 9 GDPR) – data protection officer, works council, a data protection impact assessment, and consent of
  everyone recorded. The app already makes recognition opt-in with a note and deletion per person or all.

## How to check (test PC)

- Copy the tree: `tar` over `ssh pc` into `E:\OwnApps\scripta-windows` (excluding `.git`, `node_modules`,
  `src-tauri/target`, `dist`, `src-tauri/native`), see `CLAUDE.md`.
- Helper scripts in `E:\OwnApps` run copies with another hash, because Smart App Control blocks some fresh
  builds:
  - `_e2e.ps1` (CPU) and `_e2e.ps1 -Gpu "AMD Radeon RX 7900 XTX"`: the end-to-end tests,
  - `_bench.ps1 [-Gpu <card>] [-Minutes 120]`: `examples/benchmark.rs` on `Testaufnahme\system.m4a`
    (reading, voice detector, speech, speakers),
  - `_install.ps1`: installs the newest bundle silently, keeps it as `Scripta_0.1.0_x64-setup.exe` only if
    Smart App Control let it run, starts the installed app once with a throwaway data folder,
  - `_gpuuse.ps1`: GPU use per process on the RX 7900 XTX (LUID `0x0000e868`) and the busiest processes.
- Unit tests: `cargo test --release --lib`. Speakers: `examples/calibrate.rs` with `SCRIPTA_FORCED=none`.
- Over SSH the app always stops at "failed to create webview … Ungültiges Fensterhandle" (no desktop in that
  session); the start check only means no Code Integrity event 3033/3077.
- Graphics card timings swing when the owner uses the PC (speech on the 2-hour track: 26–99 s); the processor
  parts (reading, voice detector, speakers) are stable.
- Before installing, check that Scripta isn't running.
- The owner is often at the PC: don't drive the UI; read the log
  (`%LOCALAPPDATA%\io.github.saibot-strg.scripta\Logs`) and tell the owner what to click.
- The scheduled task `ScriptaAgent` (desktop automation) still exists; delete it when it's no longer needed:
  `schtasks /delete /tn ScriptaAgent /f`.
