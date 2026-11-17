"""Debug plot tests (Milestone 10)."""

import numpy as np
import pytest
from synth import SR, make_track

pytest.importorskip("matplotlib")

import soundfile as sf

from automix.debug_plot import envelope
from automix.mix import mix_two_tracks


def test_envelope_follows_peaks():
    audio = np.zeros((2, SR), dtype=np.float32)
    audio[:, SR // 2] = 0.9
    times, upper, lower = envelope(audio, SR, points=100)
    assert len(times) == len(upper) == len(lower) == 100
    assert upper.max() == pytest.approx(0.9) and lower.min() == pytest.approx(0.0)
    assert times[int(np.argmax(upper))] == pytest.approx(0.5, abs=0.02)


def test_mix_writes_plot(tmp_path):
    a, b = tmp_path / "a.wav", tmp_path / "b.wav"
    sf.write(str(a), make_track(120, 32), SR)
    sf.write(str(b), make_track(126, 32), SR)
    png = tmp_path / "plots" / "t.png"
    mix_two_tracks(a, b, tmp_path / "o.wav", cache_dir=None, plot=png)
    assert png.exists() and png.stat().st_size > 5000
