"""Key matching tests: Camelot rules, choice of pitch shift, and the shift itself."""

import numpy as np
import pytest
import soundfile as sf
from synth import SR, make_track

from automix.analyzer.key import detect_key
from automix.camelot import best_shift, compatibility, shift_code
from automix.mix import mix_two_tracks
from automix.renderer.stretch import pitch_shift


def test_compatibility_rules():
    assert compatibility("8A", "8A") == 1.0
    assert compatibility("8A", "8B") == 0.9  # relative major
    assert compatibility("8A", "9A") == 0.9 and compatibility("8A", "7A") == 0.9
    assert compatibility("12B", "1B") == 0.9  # the wheel is a circle
    assert compatibility("8A", "9B") == 0.6
    assert compatibility("8A", "10A") == 0.5  # energy boost
    assert compatibility("8A", "2A") == 0.1  # clash


def test_shift_code():
    assert shift_code("8A", 1) == "3A"  # A minor -> Bb minor
    assert shift_code("8B", 2) == "10B"  # C major -> D major
    assert shift_code("10A", -2) == "8A"


def test_best_shift():
    assert best_shift("8A", "8A")[0] == 0
    assert best_shift("8A", "9A")[0] == 0  # already fine, do not touch
    assert best_shift("8A", "10A")[0] == -2  # 0.5 -> 1.0
    assert best_shift("8A", "3A")[0] == -1  # one semitone down -> same key
    shift, plain, shifted = best_shift("8A", "2A")
    assert plain == 0.1 and shifted >= 0.9 and abs(shift) <= 2


def test_pitch_shift_moves_a_tone():
    t = np.arange(2 * SR) / SR
    tone = (0.5 * np.sin(2 * np.pi * 440.0 * t)).astype(np.float32)[np.newaxis, :]
    out, _ = pitch_shift(tone, SR, 2)
    assert out.shape == tone.shape
    spectrum = np.abs(np.fft.rfft(out[0] * np.hanning(out.shape[1])))
    peak_hz = np.argmax(spectrum) * SR / out.shape[1]
    assert peak_hz == pytest.approx(440.0 * 2 ** (2 / 12), rel=0.01)  # 493.9 Hz


def test_mix_shifts_b_into_the_key_of_a(tmp_path):
    a, b = tmp_path / "a.wav", tmp_path / "b.wav"
    sf.write(a, make_track(120.0, 32), SR)  # C major, 8B
    sf.write(b, make_track(122.0, 32, semitones=2), SR)  # D major, 10B
    out = tmp_path / "out.wav"

    plan, result = mix_two_tracks(a, b, out, cache_dir=tmp_path / "cache", beat_backend="librosa")
    assert (plan.key_a, plan.key_b) == ("8B", "10B")
    assert plan.pitch_shift_b == -2 and plan.key_b_played == "8B"

    audio, sr = sf.read(out)
    tail = audio[int((result.transition_end + 2.0) * sr) :, 0].astype(np.float32)
    assert detect_key(tail, sr).name == "C major"

    plan, _ = mix_two_tracks(
        a, b, out, cache_dir=tmp_path / "cache", beat_backend="librosa", key_match=False
    )
    assert plan.pitch_shift_b == 0
