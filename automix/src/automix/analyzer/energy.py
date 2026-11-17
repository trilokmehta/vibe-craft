"""Loudness and energy.

* Loudness (LUFS): one number for how loud the whole track sounds to a human.
  Standard: ITU-R BS.1770. Used later to make all tracks equally loud.
* Energy curve: short-term level (RMS, in dB) over time. Shows where the song
  is quiet (intro, breakdown) and where it is loud (chorus, drop).
"""

from __future__ import annotations

import librosa
import numpy as np
import pyloudnorm

HOP = 512
FRAME = 2048


def amplitude_to_db(x: np.ndarray | float) -> np.ndarray | float:
    return 20.0 * np.log10(np.maximum(x, 1e-6))


def integrated_loudness(audio: np.ndarray, sr: int) -> float | None:
    """Integrated loudness in LUFS. `audio` has shape (channels, samples).

    Returns None when the file is too short or silent.
    """
    try:
        meter = pyloudnorm.Meter(sr)
        lufs = float(meter.integrated_loudness(audio.T.astype(np.float64)))
    except ValueError:
        return None
    return lufs if np.isfinite(lufs) else None


def peak_db(audio: np.ndarray) -> float:
    """Highest sample level in dBFS (0 dB = digital maximum)."""
    return float(amplitude_to_db(float(np.max(np.abs(audio))) if audio.size else 0.0))


def energy_curve(y: np.ndarray, sr: int, step_seconds: float = 0.5) -> tuple[float, np.ndarray]:
    """RMS level in dB, one value every `step_seconds`.

    Returns (step_seconds, values).
    """
    rms = librosa.feature.rms(y=y, frame_length=FRAME, hop_length=HOP)[0]
    frames_per_step = max(1, int(round(step_seconds * sr / HOP)))
    n_steps = int(np.ceil(len(rms) / frames_per_step))
    values = np.empty(n_steps)
    for i in range(n_steps):
        chunk = rms[i * frames_per_step : (i + 1) * frames_per_step]
        values[i] = np.sqrt(np.mean(chunk**2))
    return frames_per_step * HOP / sr, np.asarray(amplitude_to_db(values))
