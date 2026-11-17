"""Runs all analysis steps for one track and caches the result as JSON."""

from __future__ import annotations

import hashlib
import json
from dataclasses import asdict
from pathlib import Path

import librosa
import numpy as np

from ..audio_io import load_audio
from .beats import detect_beats
from .energy import energy_curve, integrated_loudness, peak_db
from .key import detect_key
from .structure import detect_phrases

# Increase this number when the analysis changes, so old cache files are ignored.
ANALYZER_VERSION = 2
ANALYSIS_SR = 22050
DEFAULT_CACHE_DIR = Path("automix_cache")


def _cache_path(path: Path, cache_dir: Path, beat_backend: str) -> Path:
    """Cache file name. Changes when the audio file or the analyzer changes."""
    stat = path.stat()
    fingerprint = (
        f"{path.resolve()}|{stat.st_size}|{stat.st_mtime_ns}|{ANALYZER_VERSION}|{beat_backend}"
    )
    digest = hashlib.sha1(fingerprint.encode("utf-8")).hexdigest()[:10]
    return cache_dir / f"{path.stem}.{digest}.json"


def _round_list(values, digits: int = 3) -> list[float]:
    return [round(float(v), digits) for v in values]


def analyze_track(
    path: str | Path,
    cache_dir: str | Path | None = DEFAULT_CACHE_DIR,
    beat_backend: str = "auto",
    use_cache: bool = True,
) -> dict:
    """Analyze one audio file. Returns a plain dict (the same data as the JSON).

    The result also has "cache_file" (where the JSON is saved, or None) and
    "from_cache" (True when the analysis was loaded and not computed).
    """
    path = Path(path)
    cache_file = _cache_path(path, Path(cache_dir), beat_backend) if cache_dir else None

    if use_cache and cache_file and cache_file.exists():
        result = json.loads(cache_file.read_text(encoding="utf-8"))
        result["cache_file"] = str(cache_file)
        result["from_cache"] = True
        return result

    # 1. Load. Loudness is measured on the original audio (stereo, full rate).
    audio, sr = load_audio(path)
    duration = audio.shape[1] / sr
    lufs = integrated_loudness(audio, sr)
    peak = peak_db(audio)

    # 2. Everything else works on a mono, 22 kHz copy (faster, and enough detail).
    y = librosa.resample(librosa.to_mono(audio), orig_sr=sr, target_sr=ANALYSIS_SR)

    grid = detect_beats(y, ANALYSIS_SR, backend=beat_backend)
    key = detect_key(y, ANALYSIS_SR)
    phrases = detect_phrases(y, ANALYSIS_SR, grid.downbeats)
    step, curve = energy_curve(y, ANALYSIS_SR)

    result = {
        "analyzer_version": ANALYZER_VERSION,
        "file": str(path.resolve()),
        "duration": round(duration, 3),
        "sample_rate": sr,
        "channels": int(audio.shape[0]),
        "bpm": round(grid.bpm, 2),
        "beat_backend": grid.backend,
        "beats": _round_list(grid.beats),
        "downbeats": _round_list(grid.downbeats),
        "key": {**asdict(key), "confidence": round(key.confidence, 3)},
        "loudness_lufs": None if lufs is None else round(lufs, 2),
        "peak_db": round(peak, 2),
        "energy": {"step_seconds": round(step, 4), "rms_db": _round_list(curve, 2)},
        "phrases": [
            {
                **asdict(p),
                "start": round(p.start, 3),
                "end": round(p.end, 3),
                "energy_db": round(p.energy_db, 2),
            }
            for p in phrases
        ],
    }

    if cache_file:
        cache_file.parent.mkdir(parents=True, exist_ok=True)
        cache_file.write_text(json.dumps(result, indent=1), encoding="utf-8")

    result["cache_file"] = str(cache_file) if cache_file else None
    result["from_cache"] = False
    return result
