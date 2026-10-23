# Speaker models: what was tried and why TitaNet small

Diarization is sherpa-onnx's offline pipeline: pyannote segmentation 3.0 finds who speaks when in short
windows, a speaker embedding model turns each stretch into a voice fingerprint, and agglomerative clustering
groups them (`threshold`: larger means fewer speakers). Per speaker, Scripta keeps the mean embedding of its
longest turns for voice recognition.

## Test material (all synthetic)

- `speakers-en.m4a` / `speakers-en-teams.m4a`: Scripta's macOS voices Samantha and Daniel (clean, and
  band-limited and compressed like a Teams call).
- `meeting-de.m4a` / `meeting-de-call.m4a`: Windows voices Hedda, Katja and Stefan in a German meeting (9 and 6
  turns, other sentences in the second one), the second one call-like (5.5 kHz band, noise, 8-bit μ-law).
  Made with `scripts/make-test-voices.ps1` and `scripts/make-test-meetings.py`.

## Results (2026-10-23, `examples/speaker_lab.rs` and `examples/calibrate.rs`)

The German numbers come from the first version of the German clips, whose text had mangled umlauts (the
script was read as ANSI); voices and lengths were the same.

Turn-level cosine similarity in the German meeting and how the speaker profiles of the clean meeting match the
call (same person vs. the most similar other person):

| Embedding model | same person (turns) | different people (turns) | same person over the call | other people over the call |
|---|---|---|---|---|
| WeSpeaker ResNet34 (VoxCeleb) | 0.92–0.96 | 0.49–0.65 | 0.72–0.89 | up to 0.65 |
| WeSpeaker ResNet152 | 0.94–0.97 | 0.60–0.79 | 0.80–0.92 | up to 0.77 |
| WeSpeaker CAM++ | 0.28–0.85 | 0.03–0.97 | unusable | |
| 3D-Speaker ERes2Net base | 0.90–0.94 | 0.15–0.50 | 0.74–0.79 | up to 0.38 |
| 3D-Speaker ERes2Net v2 | 0.91–0.97 | 0.37–0.71 | 0.79–0.82 | up to 0.59 |
| 3D-Speaker CAM++ (zh/en) | 0.92–0.96 | 0.29–0.60 | 0.69–0.88 | up to 0.49 |
| **NVIDIA TitaNet small** | 0.93–0.97 | 0.34–0.57 | 0.80–0.87 | up to 0.50 |

Whole pipeline by clustering threshold ("right": the right number of speakers and every turn attributed correctly; ResNet34 was only counted):

| Model | German clean (3) | German call (3) | Samantha/Daniel clean (2) | … call-like (2) |
|---|---|---|---|---|
| WeSpeaker ResNet34 | 3 speakers at 0.4–0.5 | 3 speakers only at 0.4 | 2 speakers at ≤ 0.4 | 2 speakers at 0.3–0.7 |
| ERes2Net base | right at 0.5–0.7 | 4–5 speakers at every threshold | | |
| CAM++ (zh/en) | right at 0.5–0.7 | right at 0.4–0.7 | right at 0.4–0.7 | right at 0.3–0.7 |
| **TitaNet small** | right at 0.3–0.6 | right at 0.3–0.7 | right at 0.4–0.7 | right at 0.4–0.7 |

**Chosen: TitaNet small with threshold 0.5**, the middle of the range where all four dialogues are separated
correctly. Its similarities also fit Scripta's recognition thresholds unchanged (suggestion from 0.5,
automatic from 0.7 with a margin of 0.1): the same person over a call scored 0.78–0.87, other people at most
0.50.

## Caveats

- Synthetic voices only. Real people differ more from each other than voices of one synthesizer, but real
  recordings have more noise, overlap and crosstalk. Calibrate on real meetings once colleagues use it; the
  values live in `src-tauri/src/engine.rs` (`CLUSTER_THRESHOLD`) and `src-tauri/src/voices.rs`.
- Changing the embedding model makes stored voice profiles useless (different vectors). Voice samples of
  another length are ignored by the matcher; the people would have to be confirmed again.
