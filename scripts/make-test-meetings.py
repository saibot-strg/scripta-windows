# Builds the German meeting test fixtures from the clips of
# make-test-voices.ps1: 0.7 s pauses between turns; the call version is
# band-limited, with a little noise and 8-bit mu-law. Prints the turns.
# python3 scripts/make-test-meetings.py <clips dir> src-tauri/tests/fixtures (macOS: uses afconvert)
import glob, os, sys, wave
import numpy as np
from scipy.signal import resample_poly

clips_dir, out_dir = sys.argv[1], sys.argv[2]
RATE = 16000

def read(path):
    with wave.open(path) as w:
        assert w.getframerate() == RATE and w.getnchannels() == 1
        return np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768

def call_like(x):
    narrow = resample_poly(resample_poly(x, 441, 640), 640, 441)[: len(x)]
    out = np.empty_like(narrow)
    prev_in = prev_out = 0.0
    rng = np.random.default_rng(1)
    noise = (rng.random(len(narrow)) - 0.5) * 0.01
    for i, v in enumerate(narrow):
        high = 0.943 * (prev_out + v - prev_in)
        prev_in, prev_out = v, high
        out[i] = high
    value = np.clip(out * 0.8 + noise, -1, 1)
    mu = 255.0
    encoded = np.sign(value) * np.log1p(mu * np.abs(value)) / np.log1p(mu)
    quantized = np.round(encoded * 127) / 127
    return (np.sign(quantized) * ((1 + mu) ** np.abs(quantized) - 1) / mu).astype(np.float32)

def dialogue(session, transform):
    audio, turns = [], []
    position = 0
    for path in sorted(glob.glob(os.path.join(clips_dir, f"{session}-*.wav"))):
        who = os.path.basename(path)[:-4].split("-")[-1]
        clip = transform(read(path))
        turns.append((position / RATE, (position + len(clip)) / RATE, who))
        audio.append(clip)
        audio.append(np.zeros(int(RATE * 0.7), dtype=np.float32))
        position += len(clip) + int(RATE * 0.7)
    return np.concatenate(audio), turns

def write(path, samples):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes((np.clip(samples, -1, 1) * 32767).astype(np.int16).tobytes())

for name, session, transform in [("meeting-de", "a", lambda x: x), ("meeting-de-call", "b", call_like)]:
    samples, turns = dialogue(session, transform)
    wav_path = os.path.join(out_dir, name + ".wav")
    write(wav_path, samples)
    m4a = os.path.join(out_dir, name + ".m4a")
    os.system(f'afconvert -f m4af -d aac -q 127 -s 0 -b 48000 "{wav_path}" "{m4a}"')
    os.remove(wav_path)
    print(name, f"{len(samples)/RATE:.1f} s")
    for start, end, who in turns:
        print(f'    ({start:.2f}, {end:.2f}, "{who}"),')
