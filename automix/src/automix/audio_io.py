"""Audio file loading.

librosa first tries `soundfile` (reads MP3, WAV, FLAC, OGG directly) and then
falls back to `audioread`, which needs FFmpeg on PATH (for M4A and others).
"""

from __future__ import annotations

from pathlib import Path

import librosa
import numpy as np

AUDIO_EXTENSIONS = {".mp3", ".wav", ".flac", ".ogg", ".m4a", ".aac", ".aiff", ".aif"}


def load_audio(path: str | Path) -> tuple[np.ndarray, int]:
    """Load a file at its original sample rate.

    Returns (audio, sample_rate). `audio` has shape (channels, samples),
    also for mono files (then channels == 1).
    """
    y, sr = librosa.load(str(path), sr=None, mono=False)
    if y.ndim == 1:
        y = y[np.newaxis, :]
    return y.astype(np.float32), int(sr)


def find_audio_files(paths: list[str]) -> list[Path]:
    """Expand a list of files and folders into a sorted list of audio files."""
    found: list[Path] = []
    for p in paths:
        path = Path(p)
        if path.is_dir():
            found.extend(
                sorted(f for f in path.iterdir() if f.suffix.lower() in AUDIO_EXTENSIONS)
            )
        elif path.is_file():
            found.append(path)
        else:
            raise FileNotFoundError(f"File or folder not found: {p}")
    return found
