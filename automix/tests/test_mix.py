"""Mixing tests: a 120 BPM track into a 126 BPM track."""

import librosa
import numpy as np
import pytest
import soundfile as sf
from synth import SR, make_track

from automix.mix import mix_two_tracks
from automix.planner.simple import plan_simple_transition
from automix.renderer.eq import band_gains, equal_power_curves, split_bands
from automix.renderer.stretch import stretch_to_map


def _fake_analysis(bpm: float, bars: int) -> dict:
    bar = 4 * 60.0 / bpm
    phrases = [
        {"index": k, "start_bar": k * 8, "bars": 8, "energy_db": -20.0, "label": "verse"}
        for k in range(bars // 8)
    ]
    return {"bpm": bpm, "downbeats": [i * bar for i in range(bars + 1)], "phrases": phrases}


def test_equal_power():
    fade_out, fade_in = equal_power_curves(1000)
    assert np.allclose(fade_out**2 + fade_in**2, 1.0, atol=1e-5)
    assert fade_out[0] == pytest.approx(1.0) and fade_in[-1] == pytest.approx(1.0)


def test_plan_same_bars():
    plan = plan_simple_transition(_fake_analysis(120, 32), _fake_analysis(126, 32))
    assert plan.beat_matched
    assert plan.bars_a == 16 and plan.bars_b == 16
    assert plan.stretch_rate == pytest.approx(120 / 126, abs=1e-6)
    assert plan.a_start == pytest.approx(32.0, abs=1e-6)  # bar 16 of 32 at 120 BPM
    assert plan.b_start == pytest.approx(0.0, abs=1e-6)
    assert len(plan.out_anchors) == 17
    assert plan.bass_swap == pytest.approx(plan.out_anchors[8])


def test_plan_double_tempo():
    plan = plan_simple_transition(_fake_analysis(140, 32), _fake_analysis(70, 32))
    assert plan.beat_matched
    assert plan.bars_b == 8
    assert plan.stretch_rate == pytest.approx(1.0, abs=1e-6)


def test_plan_tempo_too_far():
    plan = plan_simple_transition(_fake_analysis(120, 32), _fake_analysis(100, 32))
    assert not plan.beat_matched
    assert plan.stretch_rate == 1.0


def test_mix_keeps_beats_aligned(tmp_path):
    a, b = tmp_path / "a.wav", tmp_path / "b.wav"
    sf.write(a, make_track(120.0, 32), SR)
    sf.write(b, make_track(126.0, 32), SR)
    out = tmp_path / "out.wav"

    plan, result = mix_two_tracks(a, b, out, cache_dir=tmp_path / "cache", beat_backend="librosa")
    assert plan.beat_matched
    assert plan.stretch_rate == pytest.approx(120 / 126, abs=0.01)

    audio, sr = sf.read(out)
    assert sr == SR and audio.ndim == 2
    assert np.max(np.abs(audio)) <= 1.0

    # Inside the transition both kicks play while the tempo changes. Convert
    # time to "position inside the beat" with the plan's bar lines. If the
    # tracks are aligned, the hits of both sit at one position in the beat.
    first = int(result.transition_start * sr)
    last = int(result.transition_end * sr)
    middle = audio[first:last, 0].astype(np.float32)
    strength = librosa.onset.onset_strength(y=middle, sr=sr)
    times = librosa.frames_to_time(np.arange(len(strength)), sr=sr)
    bar_position = np.interp(times, plan.out_anchors, np.arange(len(plan.out_anchors)))
    angle = 2 * np.pi * bar_position * 4
    centre = np.angle(np.sum(strength * np.exp(1j * angle)))
    beat_error = np.abs(np.angle(np.exp(1j * (angle - centre)))) / (2 * np.pi)
    # Share of all "hit energy" within a tenth of a beat (about 50 ms) of the grid.
    on_grid = strength[beat_error < 0.1].sum() / strength.sum()
    assert on_grid > 0.55

    # Tempo ramp: bars get shorter, from A's bar length (2.0 s) towards B's (1.905 s).
    bar_lengths = np.diff(plan.out_anchors)
    assert bar_lengths[0] == pytest.approx(2.0, abs=0.02)
    assert bar_lengths[-1] == pytest.approx(240 / 126, abs=0.02)
    assert np.all(np.diff(bar_lengths) < 0.01)

    # After the transition B plays at its own tempo, untouched.
    tail = audio[last + sr :, 0]
    original, _ = sf.read(b)
    b_from = int(round((plan.b_end + 1.0) * sr))
    scale = 10 ** ((result.gain_b_db + result.master_gain_db) / 20)
    # Rounding may move the join by one or two samples (under 0.1 ms).
    assert any(
        np.allclose(tail[:sr], original[b_from + d : b_from + d + sr] * scale, atol=2e-3)
        for d in range(-2, 3)
    )


def test_bands_add_up_to_original():
    audio = np.random.default_rng(0).standard_normal((2, SR)).astype(np.float32) * 0.1
    bands = split_bands(audio, SR)
    assert np.allclose(bands["low"] + bands["mid"] + bands["high"], audio, atol=1e-6)

    t = np.arange(SR) / SR
    kick = np.sin(2 * np.pi * 60 * t)[np.newaxis, :]
    parts = split_bands(kick, SR)
    assert np.sqrt(np.mean(parts["low"] ** 2)) > 0.65  # a 60 Hz tone is "low"
    assert np.sqrt(np.mean(parts["high"] ** 2)) < 0.01


def test_only_one_bass_at_a_time():
    n, swap = 10 * SR, 5 * SR
    low_a, low_b = band_gains(n, SR, swap)["low"]
    assert np.all(low_a[: swap - SR // 10] == 1.0) and np.all(low_b[: swap - SR // 10] == 0.0)
    assert np.all(low_a[swap:] == 0.0) and np.all(low_b[swap:] == 1.0)
    assert np.allclose(low_a**2 + low_b**2, 1.0)
    mid_a, mid_b = band_gains(n, SR, swap)["mid"]
    assert 0.6 < mid_a[swap] < 0.8 and 0.6 < mid_b[swap] < 0.8  # mids overlap


def test_stretch_follows_time_map():
    # 4 clicks, one per second. Ask for them at 0.5, 1.4, 2.2 and 2.9 s.
    y = np.zeros((1, 4 * SR), dtype=np.float32)
    for k in range(4):
        y[0, int((k + 0.5) * SR) : int((k + 0.5) * SR) + 40] = 0.9
    src = np.array([0.0, 0.5, 1.5, 2.5, 3.5, 4.0])
    dst = np.array([0.0, 0.5, 1.4, 2.2, 2.9, 3.3])
    out, _ = stretch_to_map(y, SR, src, dst)
    assert out.shape[1] == int(round(3.3 * SR))
    for want in dst[1:5]:
        window = np.abs(out[0, int((want - 0.1) * SR) : int((want + 0.1) * SR)])
        found = (int((want - 0.1) * SR) + int(np.argmax(window))) / SR
        # The fallback stretch smears a click over one analysis window (93 ms here).
        assert found == pytest.approx(want, abs=0.06)
