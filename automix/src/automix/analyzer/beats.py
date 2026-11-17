"""Beat grid: BPM, beat times and downbeat times.

Two backends:

* "beat_this" - a neural network (Foscarin et al., ISMIR 2024). Accurate beats
  AND real downbeats. Optional, needs PyTorch.
* "librosa"   - classic dynamic-programming beat tracker (Ellis, 2007).
  Always available. It gives beats only, so the downbeats are guessed with a
  simple rule (see `_guess_downbeats`). The rule assumes 4/4 time.
"""

from __future__ import annotations

import sys
from dataclasses import dataclass

import librosa
import numpy as np

HOP = 512
N_FFT = 2048
_beat_this_broken = False  # set after the first failure in "auto" mode


@dataclass
class BeatGrid:
    bpm: float
    beats: np.ndarray  # seconds
    downbeats: np.ndarray  # seconds, a subset of `beats`
    backend: str


def detect_beats(y: np.ndarray, sr: int, backend: str = "auto") -> BeatGrid:
    """Find beats and downbeats in a mono signal.

    backend: "auto" (beat_this if installed, else librosa), "beat_this", "librosa".
    """
    if backend not in ("auto", "beat_this", "librosa"):
        raise ValueError(f"Unknown beat backend: {backend}")

    global _beat_this_broken
    if backend == "beat_this" or (backend == "auto" and not _beat_this_broken):
        try:
            return _beats_beat_this(y, sr)
        except ImportError:
            if backend == "beat_this":
                raise RuntimeError(
                    "beat_this is not installed. See requirements-optional.txt."
                )
            _beat_this_broken = True
        except Exception as exc:  # model download failed, out of memory, ...
            if backend == "beat_this":
                raise
            # Do not try again for every song in this run.
            _beat_this_broken = True
            print(f"  warning: beat_this failed ({exc}); using librosa.", file=sys.stderr)

    return _beats_librosa(y, sr)


def _bpm_from_beats(beats: np.ndarray) -> float:
    """Tempo from a straight line fitted through all beat times.

    The time between two beats is rounded to the analysis frame (about 23 ms),
    so a single gap is not exact (120 BPM can look like 117). A line through
    all beats averages this rounding away. A missed beat counts as 2 steps.
    """
    if len(beats) < 2:
        return 0.0
    gaps = np.diff(beats)
    median_gap = float(np.median(gaps))
    if len(beats) < 4 or median_gap <= 0:
        return 60.0 / median_gap if median_gap > 0 else 0.0
    steps = np.maximum(1, np.round(gaps / median_gap))
    index = np.concatenate([[0.0], np.cumsum(steps)])
    seconds_per_beat = float(np.polyfit(index, beats, 1)[0])
    return 60.0 / seconds_per_beat if seconds_per_beat > 0 else 0.0


def _beats_beat_this(y: np.ndarray, sr: int) -> BeatGrid:
    from beat_this.inference import Audio2Beats  # optional dependency

    model = Audio2Beats(checkpoint_path="final0", device="cpu", dbn=False)
    beats, downbeats = model(y, sr)
    beats = np.asarray(beats, dtype=float)
    downbeats = np.asarray(downbeats, dtype=float)
    return BeatGrid(_bpm_from_beats(beats), beats, downbeats, "beat_this")


def _beats_librosa(y: np.ndarray, sr: int) -> BeatGrid:
    onset_env = librosa.onset.onset_strength(y=y, sr=sr, hop_length=HOP)
    _, beat_frames = librosa.beat.beat_track(
        onset_envelope=onset_env, sr=sr, hop_length=HOP, units="frames"
    )
    beat_frames = np.asarray(beat_frames, dtype=int)
    beats = librosa.frames_to_time(beat_frames, sr=sr, hop_length=HOP)
    downbeats = _guess_downbeats(y, sr, beat_frames, beats)
    return BeatGrid(_bpm_from_beats(beats), beats, downbeats, "librosa")


def _zscore(x: np.ndarray) -> np.ndarray:
    std = x.std()
    return (x - x.mean()) / std if std > 1e-9 else np.zeros_like(x)


def _guess_downbeats(
    y: np.ndarray, sr: int, beat_frames: np.ndarray, beats: np.ndarray
) -> np.ndarray:
    """Guess which beat is "beat 1" of each bar. Assumes 4/4 time.

    There are 4 possible answers (the bar starts on beat 0, 1, 2 or 3).
    We score each one with two clues and keep the best:

    1. Bass hits: the kick drum / bass note is usually strongest on beat 1.
    2. Chord changes: harmony usually changes at the start of a bar.
    """
    if len(beat_frames) < 8:
        return beats[::4]

    spec = np.abs(librosa.stft(y, n_fft=N_FFT, hop_length=HOP))
    n_frames = spec.shape[1]
    freqs = librosa.fft_frequencies(sr=sr, n_fft=N_FFT)

    # Clue 1: how much the low band (< 150 Hz) jumps up at each beat.
    low = spec[freqs < 150.0].sum(axis=0)
    low_flux = np.maximum(0.0, np.diff(low, prepend=low[0]))
    bass_hit = np.array(
        [low_flux[max(0, f - 2) : min(n_frames, f + 3)].max() for f in beat_frames]
    )

    # Clue 2: how different the notes are before and after each beat.
    chroma = librosa.feature.chroma_stft(S=spec**2, sr=sr)
    edges = np.append(beat_frames, n_frames)
    beat_chroma = np.stack(
        [
            chroma[:, a : max(b, a + 1)].mean(axis=1)
            for a, b in zip(edges[:-1], edges[1:])
        ]
    )
    norms = np.linalg.norm(beat_chroma, axis=1) + 1e-9
    unit = beat_chroma / norms[:, None]
    chord_change = np.zeros(len(beat_frames))
    chord_change[1:] = 1.0 - np.sum(unit[1:] * unit[:-1], axis=1)

    clue = _zscore(bass_hit) + _zscore(chord_change)
    scores = [clue[phase::4].mean() for phase in range(4)]
    best_phase = int(np.argmax(scores))
    return beats[best_phase::4]
