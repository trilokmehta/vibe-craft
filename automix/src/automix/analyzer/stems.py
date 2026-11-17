"""Vocal stem: where does somebody sing?

Demucs (Défossez et al., MIT license) is a neural network that splits a song
into drums, bass, other and vocals. We keep only the vocals:

* a FLAC file with the vocal stem (the renderer uses it to turn one singer
  down while the other sings),
* a number per bar, 0 = no singing, 1 = full singing (the planner uses it to
  avoid transitions where two people sing at once).

Separation is slow on a CPU (often one to three minutes per song), so the
result is cached next to the normal analysis.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import librosa
import numpy as np
import soundfile as sf

from ..audio_io import load_audio

MODEL_NAME = "htdemucs"
STEMS_VERSION = 1
# A bar counts as "full singing" when its vocal level is within 5 dB of the
# loud vocal bars of the track, and as "no singing" 15 dB below them.
FULL_DB = 5.0
NONE_DB = 15.0
# If even the loud vocal bars are this far below the whole mix, the track is
# treated as instrumental.
INSTRUMENTAL_DB = 25.0

_model = None


def demucs_available() -> bool:
    try:
        import demucs  # noqa: F401
    except ImportError:
        return False
    return True


def separate_vocals(audio: np.ndarray, sr: int, progress: bool = True) -> np.ndarray:
    """Run Demucs. `audio` is (channels, samples); returns the vocal stem
    with the same shape and sample rate."""
    import torch
    from demucs.apply import apply_model
    from demucs.pretrained import get_model

    global _model
    if _model is None:
        _model = get_model(MODEL_NAME)
        _model.eval()

    n = audio.shape[1]
    stereo = np.repeat(audio, 2, axis=0) if audio.shape[0] == 1 else audio[:2]
    if sr != _model.samplerate:
        stereo = librosa.resample(stereo, orig_sr=sr, target_sr=_model.samplerate)

    wav = torch.from_numpy(np.ascontiguousarray(stereo, dtype=np.float32))
    # Demucs expects audio with zero mean and unit loudness.
    ref = wav.mean(0)
    mean, std = ref.mean(), ref.std() + 1e-8
    with torch.no_grad():
        sources = apply_model(
            _model, ((wav - mean) / std)[None], device="cpu", split=True, overlap=0.25,
            progress=progress,
        )[0]
    vocals = (sources[_model.sources.index("vocals")] * std + mean).numpy()

    if sr != _model.samplerate:
        vocals = librosa.resample(vocals, orig_sr=_model.samplerate, target_sr=sr)
    vocals = vocals[:, :n] if vocals.shape[1] >= n else np.pad(vocals, ((0, 0), (0, n - vocals.shape[1])))
    return vocals[: audio.shape[0]] if audio.shape[0] == 1 else vocals


def _bar_db(audio: np.ndarray, sr: int, downbeats: np.ndarray) -> np.ndarray:
    mono = audio.mean(axis=0)
    edges = np.clip((np.asarray(downbeats) * sr).astype(int), 0, len(mono))
    out = np.full(len(edges) - 1, -120.0)
    for k, (first, last) in enumerate(zip(edges[:-1], edges[1:])):
        if last > first:
            out[k] = 10.0 * np.log10(max(float(np.mean(mono[first:last] ** 2)), 1e-12))
    return out


def bar_activity(vocals: np.ndarray, mix: np.ndarray, sr: int, downbeats) -> np.ndarray:
    """Singing per bar, 0..1, from the vocal stem and the full mix."""
    if len(downbeats) < 2:
        return np.zeros(0)
    vocal_db = _bar_db(vocals, sr, downbeats)
    mix_db = _bar_db(mix, sr, downbeats)
    loud_vocal = np.percentile(vocal_db, 90)
    if loud_vocal < np.percentile(mix_db, 90) - INSTRUMENTAL_DB:
        return np.zeros(len(vocal_db))
    return np.clip((vocal_db - (loud_vocal - NONE_DB)) / (NONE_DB - FULL_DB), 0.0, 1.0)


def vocal_analysis(path: str | Path, analysis: dict, progress: bool = True) -> dict:
    """Vocal data for one analyzed track: {"bar_activity": [...], "file": "..."}.

    Needs the normal analysis first (for its bar lines and its cache file).
    The vocal stem is saved as FLAC next to the analysis JSON.
    """
    path = Path(path)
    cache_file = analysis.get("cache_file")
    if not cache_file:
        raise ValueError("Vocal analysis needs the analysis cache (do not use it without a cache folder).")
    base = Path(cache_file).with_suffix("")
    json_file = base.with_name(base.name + ".vocals.json")
    flac_file = base.with_name(base.name + ".vocals.flac")

    if json_file.exists() and flac_file.exists():
        data = json.loads(json_file.read_text(encoding="utf-8"))
        if data.get("stems_version") == STEMS_VERSION:
            return data

    if progress:
        print(f"  separating vocals: {path.name} (slow the first time)", file=sys.stderr)
    audio, sr = load_audio(path)
    vocals = separate_vocals(audio, sr, progress=progress)
    sf.write(str(flac_file), np.clip(vocals.T, -1.0, 1.0), sr, format="FLAC", subtype="PCM_16")

    data = {
        "stems_version": STEMS_VERSION,
        "model": MODEL_NAME,
        "file": str(flac_file),
        "bar_activity": [round(float(v), 3) for v in bar_activity(vocals, audio, sr, analysis["downbeats"])],
    }
    json_file.write_text(json.dumps(data), encoding="utf-8")
    return data


def load_vocals(vocal_data: dict) -> tuple[np.ndarray, int]:
    """Read the cached vocal stem. Returns ((channels, samples), sample rate)."""
    audio, sr = sf.read(vocal_data["file"], dtype="float32", always_2d=True)
    return audio.T, int(sr)
