"""Computer-made test track, so the tests need no music files.

4/4, a kick on every beat (louder on beat 1) and a C major chord.
The first 8 bars are quiet (intro), the rest is loud.
"""

import numpy as np

SR = 22050


def make_track(bpm: float = 120.0, bars: int = 32, semitones: int = 0) -> np.ndarray:
    beat = 60.0 / bpm
    n = int(bars * 4 * beat * SR)
    t = np.arange(n) / SR
    y = np.zeros(n)

    # C major chord (C4, E4, G4), a new note attack on every bar.
    # `semitones` moves the chord up or down (2 = D major).
    up = 2.0 ** (semitones / 12.0)
    chord = sum(np.sin(2 * np.pi * f * up * t) for f in (261.63, 329.63, 392.00)) / 3.0
    bar_phase = (t % (4 * beat)) / (4 * beat)
    y += 0.25 * chord * np.exp(-2.0 * bar_phase)

    # Kick drum: a short 60 Hz thump, strongest on beat 1.
    kick_len = int(0.12 * SR)
    kt = np.arange(kick_len) / SR
    kick = np.sin(2 * np.pi * 60.0 * kt) * np.exp(-30.0 * kt)
    for b in range(bars * 4):
        start = int(b * beat * SR)
        gain = 0.9 if b % 4 == 0 else 0.35
        y[start : start + kick_len] += gain * kick[: n - start]

    # Quiet intro: first 8 bars at -12 dB.
    intro_end = int(8 * 4 * beat * SR)
    y[:intro_end] *= 0.25
    return (0.8 * y / np.max(np.abs(y))).astype(np.float32)
