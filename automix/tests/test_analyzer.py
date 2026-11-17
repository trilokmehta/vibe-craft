"""Analyzer tests on a computer-made track (see synth.py)."""

from pathlib import Path

import numpy as np
import pytest
import soundfile as sf
from synth import SR, make_track

from automix.analyzer import analyze_track
from automix.analyzer.key import key_from_chroma, MAJOR_PROFILE, MINOR_PROFILE
from automix.camelot import camelot_code

BPM = 120.0
BARS = 32


@pytest.fixture(scope="module")
def result(tmp_path_factory):
    folder = tmp_path_factory.mktemp("audio")
    wav = folder / "synthetic.wav"
    sf.write(wav, make_track(BPM, BARS), SR)
    return analyze_track(wav, cache_dir=folder / "cache", beat_backend="librosa")


def test_camelot_codes():
    assert camelot_code(0, "major") == "8B"  # C major
    assert camelot_code(9, "minor") == "8A"  # A minor
    assert camelot_code(11, "major") == "1B"  # B major
    assert camelot_code(8, "minor") == "1A"  # Ab minor
    assert camelot_code(4, "major") == "12B"  # E major


def test_key_profiles_match_themselves():
    assert key_from_chroma(np.roll(MAJOR_PROFILE, 7)).name == "G major"
    assert key_from_chroma(np.roll(MINOR_PROFILE, 2)).name == "D minor"


def test_bpm(result):
    assert result["bpm"] == pytest.approx(BPM, abs=1.5)


def test_downbeats_fall_on_bar_lines(result):
    bar = 4 * 60.0 / BPM
    downbeats = np.array(result["downbeats"])
    assert len(downbeats) >= BARS - 3
    offsets = np.abs((downbeats + bar / 2) % bar - bar / 2)
    assert np.median(offsets) < 0.07


def test_key(result):
    assert result["key"]["name"] == "C major"
    assert result["key"]["camelot"] == "8B"


def test_phrases(result):
    phrases = result["phrases"]
    assert len(phrases) >= 3
    assert phrases[0]["label"] == "intro"
    assert any(p["label"] == "chorus" for p in phrases)
    # The jump from quiet to loud at bar 8 (16 s) must be a phrase line.
    assert any(abs(p["start"] - 16.0) < 0.6 for p in phrases)


def test_loudness_and_cache(result):
    assert -40.0 < result["loudness_lufs"] < 0.0
    cache_dir = Path(result["cache_file"]).parent
    again = analyze_track(result["file"], cache_dir=cache_dir, beat_backend="librosa")
    assert again["from_cache"] is True
    assert again["bpm"] == result["bpm"]
