"""Musical key detection with the Krumhansl-Schmuckler method.

How it works:
1. Measure how much energy each of the 12 notes (C, C#, D, ...) has over the
   whole song. This is called a "chroma" profile.
2. Compare it with 24 reference profiles (12 major keys + 12 minor keys) from
   Krumhansl & Kessler (1982). The best match is the key.

The result is also given in Camelot notation (8A, 8B, ...), which DJs use:
keys with the same number, or numbers next to each other, sound good together.
"""

from __future__ import annotations

from dataclasses import dataclass

import librosa
import numpy as np

from ..camelot import camelot_code, compatibility

NOTE_NAMES = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"]

# Krumhansl-Kessler key profiles, starting at the tonic (the "home" note).
MAJOR_PROFILE = np.array(
    [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88]
)
MINOR_PROFILE = np.array(
    [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17]
)


@dataclass
class KeyResult:
    tonic: str  # e.g. "A"
    mode: str  # "major" or "minor"
    name: str  # e.g. "A minor"
    camelot: str  # e.g. "8A"
    confidence: float  # 0..1, how clearly this key beats the keys that would clash with it


def key_from_chroma(chroma_profile: np.ndarray) -> KeyResult:
    """Pick the best of the 24 keys for a 12-value note-energy profile."""
    scores: list[tuple[float, int, str]] = []
    for tonic in range(12):
        for mode, profile in (("major", MAJOR_PROFILE), ("minor", MINOR_PROFILE)):
            corr = np.corrcoef(chroma_profile, np.roll(profile, tonic))[0, 1]
            scores.append((float(corr), tonic, mode))
    scores.sort(reverse=True)
    best, tonic, mode = scores[0]
    camelot = camelot_code(tonic, mode)
    # Confidence for mixing: a mix-up with a neighbour key (for example the
    # relative major/minor) does no harm, because neighbours sound good
    # together. So we compare with the best key that is NOT a neighbour.
    rivals = [c for c, t, m in scores[1:] if compatibility(camelot, camelot_code(t, m)) < 0.9]
    confidence = float(np.clip(best - max(rivals), 0.0, 1.0))
    return KeyResult(
        tonic=NOTE_NAMES[tonic],
        mode=mode,
        name=f"{NOTE_NAMES[tonic]} {mode}",
        camelot=camelot,
        confidence=confidence,
    )


def detect_key(y: np.ndarray, sr: int) -> KeyResult:
    """Detect the key of a mono signal."""
    # Remove drums first: they add noise to every note and confuse the result.
    y_harmonic = librosa.effects.harmonic(y)
    # Many recordings are not tuned exactly to A = 440 Hz. Measure the offset
    # so that notes do not fall between two semitones.
    tuning = librosa.estimate_tuning(y=y_harmonic, sr=sr)
    chroma = librosa.feature.chroma_cqt(y=y_harmonic, sr=sr, tuning=tuning)
    return key_from_chroma(chroma.mean(axis=1))
