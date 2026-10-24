# Scripta for Windows

Scripta writes down what is said in your meetings and voice notes – locally on your PC. It records your
microphone or a whole Teams meeting, turns speech into text, tells the speakers apart and, if you want, learns
who they are. Nothing leaves your PC: no cloud, no account, no telemetry.

It is the Windows version of [Scripta for the Mac](https://github.com/saibot-strg/scripta).

---

## Install (about 5 minutes)

You need Windows 10 (version 2004 or later) or Windows 11 and about 1.5 GB of free disk space. You do **not**
need administrator rights.

Windows shows its buttons in your language; the German labels are given in brackets.

### 1. Download

Open the [latest release](https://github.com/saibot-strg/scripta-windows/releases/latest) and click the file
**`Scripta_…_x64-setup.exe`** under *Assets*.

> **Edge says the file "isn't commonly downloaded"?** (*„wird nicht häufig heruntergeladen“*)
> Point at the download, click **…** → **Keep** (*Beibehalten*) → **Show more** (*Mehr anzeigen*) →
> **Keep anyway** (*Trotzdem beibehalten*).

### 2. Run the installer

Open the downloaded file (in Edge: **Open file** / *Datei öffnen*, or double-click it in the Downloads folder).

> **A blue window says "Windows protected your PC"?** (*„Der Computer wurde durch Windows geschützt“*)
> Click **More info** (*Weitere Informationen*) and then **Run anyway** (*Trotzdem ausführen*).
> Windows shows this for every program that isn't code-signed yet; Scripta is safe to run.

Click through the installer and finish with **Finish** (*Fertig stellen*). Scripta starts right away.

### 3. First start

Read the welcome screen and click **Get Started** (*Los geht's*). Scripta now downloads its speech models once
(about 720 MB). Progress shows at the bottom left; you can already look around. After that, Scripta works
completely offline.

From now on you find **Scripta** in the Start menu.

### If Windows blocks Scripta completely

On some new PCs **Smart App Control** (*Intelligenter App-Schutz*) is turned on. It blocks programs that are
not code-signed, without a "Run anyway" button. Until Scripta is signed it can't run on those PCs. Please
don't turn Smart App Control off yourself (it can't be turned on again); ask whoever looks after your PC.

---

## Using Scripta

**Record a voice note:** click **Record** (*Aufnahme*), speak, click the red **Stop** button.

**Record a meeting** (Teams, Zoom, Webex, a browser …): click **Meeting**. Scripta records your microphone
("Me" / *Ich*) and what the PC plays ("Others" / *Andere*) separately. With the small arrow next to
**Meeting** you can record one app only, e.g. *Microsoft Teams*. Wear a headset for the cleanest result.

**Transcribe:** select a recording and click **Transcribe** (*Transkribieren*). It runs in the background;
a one-hour meeting takes a few minutes on a fast PC and longer on a laptop. With a graphics card (AMD, NVIDIA
or Intel), turn on Settings → Transcription → **Use graphics card** (*Grafikkarte verwenden*): speech
recognition then runs several times faster with a more accurate model, which is downloaded once (2.5 GB).

**Name the speakers:** above the transcript, click **Assign…** (*Zuordnen …*), listen to each voice and type
a name. If you turn on *Recognize voices* (*Stimmen wiedererkennen*), Scripta suggests the names in later
meetings and, after about three confirmed meetings, sets them by itself.

**Fix mistakes:** double-click text to correct it. Click a speaker's name above a bubble to move just that
bubble to someone else. If Scripta heard too many or too few people, use *Correct the number of people* in
the **Assign…** window.

**Import files:** drag audio or video files into the window (or click **Import**).

**Export:** **Export** above the transcript saves it as text, Markdown, subtitles (SRT, WebVTT) or JSON;
Ctrl+Shift+C copies it.

| Keyboard shortcut | Does |
| --- | --- |
| Ctrl+N | new recording |
| Ctrl+Shift+N | record a meeting |
| Ctrl+Enter | stop recording |
| Ctrl+I | import files |
| Ctrl+F | search titles and transcripts |
| Space | play / pause |

### Something doesn't work?

- **The microphone stays silent:** Windows Settings → Privacy & security → Microphone
  (*Datenschutz und Sicherheit → Mikrofon*): turn on *Let desktop apps access your microphone*.
- **"Others" stays silent in a meeting:** choose *All system audio* (*Gesamtes Systemaudio*) with the arrow
  next to **Meeting** instead of a single app.
- **Logs for support:** Settings → Privacy → *Open folder* (*Protokolle*). They contain no transcripts,
  names or voice data.

---

## Update and uninstall

- **Update:** Scripta tells you when a new version is out (bottom left). Download the new installer and run
  it; your recordings stay.
- **Uninstall:** Windows Settings → Apps → Scripta → Uninstall (*Deinstallieren*). Tick *Delete the
  application data* (*Anwendungsdaten löschen*) to remove your recordings, transcripts and voice profiles too.

## Privacy

- Recordings, transcripts, voice profiles and settings stay in
  `%LOCALAPPDATA%\io.github.saibot-strg.scripta` on your PC (Settings → Recording can move new recordings
  elsewhere).
- Voice profiles are biometric data. Recognition is off until you turn it on, and Settings → People deletes
  single people or everything.
- The only connections to the internet: the one-time model download from Hugging Face and, if enabled, a
  daily check for a new version on GitHub.
- **Ask everyone before you record.** In Germany, recording a conversation without consent is illegal. In a
  company, the data protection officer and the works council decide about using Scripta, especially voice
  recognition (GDPR Art. 9).

---

## For developers

Prerequisites: Rust (stable, MSVC toolchain), Visual Studio 2022 Build Tools with "Desktop development with
C++", Node.js 22 or later, Git.

```powershell
scripts\build-sherpa-directml.ps1                  # once (~20 min): sherpa-onnx with DirectML into src-tauri\native
npm ci
npx tauri build --target x86_64-pc-windows-msvc   # installer in src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis
```

sherpa-onnx's prebuilt Windows libraries can't use AMD or Intel graphics cards, so the script builds it from
source with ONNX Runtime's DirectML; the installer ships `onnxruntime.dll`, `DirectML.dll` and the Visual C++
runtime next to `scripta.exe`. The explicit target is needed: the app and sherpa-onnx use the static C runtime,
and `src-tauri/.cargo/config.toml` applies `+crt-static` to the app only this way.

```powershell
cd src-tauri
cargo test --lib                                              # unit tests, no models needed
cargo run --release --example models                          # download the models once
cargo test --release --test end_to_end -- --test-threads=1    # speech pipeline with synthetic voices
```

`SCRIPTA_GPU=1` runs the end-to-end tests and the tools in `src-tauri/examples` on the graphics card
(`benchmark`, and `calibrate` for speaker separation on a real recording).

- `npm run dev` on any OS opens the UI in a browser at <http://localhost:1420> with sample data
  (`src/mock.ts`), e.g. `?select=0`, `?theme=dark`, `?recording=meeting`, `?settings=people`.
- Releases: push a tag like `v0.1.0` (matching `version` in `src-tauri/tauri.conf.json`); GitHub Actions
  builds the installer and publishes it (`.github/workflows/release.yml`). Code signing:
  [docs/DISTRIBUTION.md](docs/DISTRIBUTION.md).
- Test audio is synthetic only (`scripts/make-test-voices.ps1`, `scripts/make-test-meetings.py`).
- Architecture, decisions and status: [docs/HANDOVER.md](docs/HANDOVER.md), [docs/STATUS.md](docs/STATUS.md),
  [docs/speaker-models.md](docs/speaker-models.md).

## Credits

- [NVIDIA Parakeet TDT 0.6B v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) (CC BY 4.0), ONNX export by
  [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) (Apache 2.0)
- [NVIDIA TitaNet small](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/nemo/models/titanet_small) (CC BY 4.0)
- [pyannote segmentation 3.0](https://huggingface.co/pyannote/segmentation-3.0) (MIT)
- [Silero VAD](https://github.com/snakers4/silero-vad) (MIT)
- [ONNX Runtime](https://onnxruntime.ai) (MIT) with
  [DirectML](https://github.com/microsoft/DirectML) (Microsoft Software License Terms, redistributable),
  Microsoft Visual C++ runtime (redistributable), [Tauri](https://tauri.app) (MIT/Apache 2.0),
  [Svelte](https://svelte.dev) (MIT), [Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons) (MIT)
