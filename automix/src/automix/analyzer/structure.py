"""Phrases and sections.

Most dance and pop music is built from blocks of 8 bars ("phrases"). Things
change at the start of a phrase: drums come in, the chorus starts, and so on.
A DJ starts and ends a transition on these points.

Steps:
1. Cut the song into bars using the downbeats.
2. Describe the sound of every bar (loudness, tone colour, notes).
3. Measure how much the sound changes at every bar line ("novelty", after
   Foote, 2000).
4. Phrase lines repeat every 8 bars, so there are only 8 possible grids.
   Pick the grid whose lines fall on the biggest changes.
5. Give each phrase a label from its loudness. This is a simple rule, not a
   real chorus detector:
     - "chorus"    = loud part (chorus or drop)
     - "verse"     = medium part
     - "breakdown" = quiet part in the middle
     - "intro" / "outro" = quiet part at the start / end
"""

from __future__ import annotations

from dataclasses import dataclass

import librosa
import numpy as np

HOP = 512
N_FFT = 2048
BARS_PER_PHRASE = 8


@dataclass
class Phrase:
    index: int
    start: float  # seconds
    end: float  # seconds
    start_bar: int  # index into the downbeat list
    bars: int
    energy_db: float
    label: str


def _bar_features(y: np.ndarray, sr: int, downbeats: np.ndarray):
    """Per-bar feature vectors and per-bar mean power."""
    spec = np.abs(librosa.stft(y, n_fft=N_FFT, hop_length=HOP))
    n_frames = spec.shape[1]
    rms = librosa.feature.rms(S=spec, frame_length=N_FFT)[0]
    mel = librosa.feature.melspectrogram(S=spec**2, sr=sr, n_mels=64)
    mfcc = librosa.feature.mfcc(S=librosa.power_to_db(mel), n_mfcc=13)
    chroma = librosa.feature.chroma_stft(S=spec**2, sr=sr)

    frames = librosa.time_to_frames(downbeats, sr=sr, hop_length=HOP)
    frames = np.clip(frames, 0, n_frames - 1)

    features, power = [], []
    for a, b in zip(frames[:-1], frames[1:]):
        b = max(b, a + 1)
        p = float(np.mean(rms[a:b] ** 2))
        power.append(p)
        features.append(
            np.concatenate(
                [
                    mfcc[:, a:b].mean(axis=1),
                    chroma[:, a:b].mean(axis=1),
                    [10.0 * np.log10(max(p, 1e-12))],
                ]
            )
        )
    return np.array(features), np.array(power)


def _novelty(features: np.ndarray, context: int = 4) -> np.ndarray:
    """How much the sound changes at the start of each bar.

    Compares the average of the 4 bars before with the 4 bars after.
    """
    n = len(features)
    std = features.std(axis=0)
    std[std < 1e-9] = 1.0
    f = (features - features.mean(axis=0)) / std
    novelty = np.zeros(n)
    for i in range(1, n):
        before = f[max(0, i - context) : i].mean(axis=0)
        after = f[i : min(n, i + context)].mean(axis=0)
        novelty[i] = np.linalg.norm(after - before)
    return novelty


def _best_phrase_offset(novelty: np.ndarray) -> int:
    """Which bar (0..7) starts the first full phrase."""
    n = len(novelty)
    best_offset, best_score = 0, -1.0
    for offset in range(min(BARS_PER_PHRASE, n)):
        idx = np.arange(offset, n, BARS_PER_PHRASE)
        idx = idx[idx > 0]  # bar 0 has no "before", so it has no novelty
        score = float(novelty[idx].mean()) if len(idx) else 0.0
        if score > best_score:
            best_offset, best_score = offset, score
    return best_offset


def _label_phrases(energy_db: np.ndarray) -> list[str]:
    n = len(energy_db)
    if n == 0:
        return []
    loud = np.percentile(energy_db, 90)
    rel = energy_db - loud  # 0 dB = as loud as the loudest parts

    labels = []
    for r in rel:
        if r >= -2.0:
            labels.append("chorus")
        elif r < -8.0:
            labels.append("breakdown")
        else:
            labels.append("verse")

    # Quiet phrases at the very start are the intro, at the very end the outro.
    edge_limit = max(1, n // 4)
    for i in range(min(edge_limit, n)):
        if rel[i] < -4.0:
            labels[i] = "intro"
        else:
            break
    for i in range(n - 1, max(n - 1 - edge_limit, -1), -1):
        if rel[i] < -4.0 and labels[i] != "intro":
            labels[i] = "outro"
        else:
            break
    return labels


def detect_phrases(y: np.ndarray, sr: int, downbeats: np.ndarray) -> list[Phrase]:
    """Split a track into 8-bar phrases with a section label each."""
    downbeats = np.asarray(downbeats, dtype=float)
    n_bars = len(downbeats) - 1
    if n_bars < 1:
        return []

    features, power = _bar_features(y, sr, downbeats)
    offset = _best_phrase_offset(_novelty(features)) if n_bars > BARS_PER_PHRASE else 0

    starts = list(range(offset, n_bars, BARS_PER_PHRASE))
    if offset > 0:
        starts.insert(0, 0)  # short pickup phrase before the first full one
    ends = starts[1:] + [n_bars]

    energy_db = np.array(
        [10.0 * np.log10(max(float(power[a:b].mean()), 1e-12)) for a, b in zip(starts, ends)]
    )
    labels = _label_phrases(energy_db)

    return [
        Phrase(
            index=i,
            start=float(downbeats[a]),
            end=float(downbeats[b]),
            start_bar=a,
            bars=b - a,
            energy_db=float(energy_db[i]),
            label=labels[i],
        )
        for i, (a, b) in enumerate(zip(starts, ends))
    ]
