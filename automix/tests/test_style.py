"""Style tests: automatic choice, and rendering of every style."""

import numpy as np
import pytest
import soundfile as sf
from synth import SR, make_track
from test_smart import LOUD, QUIET, fake

from automix.mix import mix_two_tracks
from automix.planner.style import plan_transition
from automix.renderer.eq import sweep_highpass


def test_auto_blend_when_b_has_a_calm_intro():
    plan = plan_transition(fake([LOUD] * 8), fake([QUIET, QUIET, LOUD, LOUD, LOUD, LOUD]))
    assert plan.style == "blend"


def test_auto_filter_when_both_busy_and_keys_fit():
    plan = plan_transition(fake([LOUD] * 8), fake([LOUD] * 6, key="9A"))
    assert plan.style == "filter" and plan.beat_matched


def test_auto_cut_when_both_busy_and_keys_clash():
    # 8A -> 2A clashes, and the key of B is not sure enough to pitch-shift.
    plan = plan_transition(fake([LOUD] * 8), fake([LOUD] * 6, key="2A", confidence=0.0))
    assert plan.style == "cut"
    assert plan.bars_a == 0 and plan.pitch_shift_b == 0
    assert plan.a_start == pytest.approx(128.0)  # A plays to its end
    assert plan.b_start == pytest.approx(0.0)


def test_auto_echo_when_tempos_are_far_apart():
    plan = plan_transition(fake([LOUD] * 8), fake([LOUD] * 6, bar=2.4))  # 120 -> 100 BPM
    assert plan.style == "echo"
    assert plan.echo_beat == pytest.approx(0.5)


def test_cut_prefers_a_loud_entry_over_a_quiet_one():
    # A is loud at its end. B has a long quiet intro: jumping into it would
    # be a big energy drop, so B should start where it gets loud.
    plan = plan_transition(
        fake([LOUD] * 8), fake([QUIET, QUIET, LOUD, LOUD, LOUD, LOUD, LOUD, LOUD]), style="cut"
    )
    assert plan.bar_b == 16


def test_unknown_style_is_rejected():
    with pytest.raises(ValueError):
        plan_transition(fake([LOUD] * 8), fake([LOUD] * 6), style="scratch")


def test_filter_sweep_removes_bass_at_the_end():
    t = np.arange(4 * SR) / SR
    bass = np.sin(2 * np.pi * 60 * t)[np.newaxis, :].astype(np.float32)
    out = sweep_highpass(bass, SR)
    rms = lambda x: float(np.sqrt(np.mean(x**2)))
    assert rms(out[0, SR // 4 : SR // 2]) > 0.5  # start: bass still there
    assert rms(out[0, -SR // 2 : -SR // 4]) < 0.02  # end: bass gone

    air = np.sin(2 * np.pi * 9000 * t)[np.newaxis, :].astype(np.float32)
    assert rms(sweep_highpass(air, SR)[0, -SR // 2 : -SR // 4]) > 0.5  # highs stay


@pytest.fixture(scope="module")
def tracks(tmp_path_factory):
    folder = tmp_path_factory.mktemp("styles")
    sf.write(folder / "a.wav", make_track(120.0, 32), SR)
    sf.write(folder / "b.wav", make_track(124.0, 32), SR)
    return folder


@pytest.mark.parametrize("style", ["blend", "filter", "cut", "echo"])
def test_every_style_renders(tracks, style):
    out = tracks / f"{style}.wav"
    plan, result = mix_two_tracks(
        tracks / "a.wav", tracks / "b.wav", out, cache_dir=tracks / "cache",
        beat_backend="librosa", style=style,
    )
    assert plan.style == style and result.style == style
    audio, sr = sf.read(out)
    assert np.isfinite(audio).all() and np.max(np.abs(audio)) <= 1.0
    assert 0 < result.transition_start <= result.transition_end < len(audio) / sr
    if style in ("cut", "echo"):
        assert result.bass_swap is None and result.stretch_engine == "none"
        b_len = sf.info(tracks / "b.wav").frames / SR
        assert len(audio) / sr == pytest.approx(plan.a_start + b_len - plan.b_start, abs=0.01)


def test_echo_adds_sound_after_the_cut(tracks):
    results = {}
    for style in ("cut", "echo"):
        _, result = mix_two_tracks(
            tracks / "a.wav", tracks / "b.wav", tracks / "x.wav", cache_dir=tracks / "cache",
            beat_backend="librosa", style=style,
        )
        results[style] = result
    cut, echo = results["cut"], results["echo"]
    assert echo.transition_end > echo.transition_start == cut.transition_start
    first = int(cut.transition_start * SR) + SR // 20
    last = int(echo.transition_end * SR)
    scale = 10 ** ((cut.master_gain_db - echo.master_gain_db) / 20)
    difference = echo.audio[:, first:last] * scale - cut.audio[:, first:last]
    # The echoes are there. (Quiet here: the test track's last beat is mostly
    # a 60 Hz kick, and echoes carry no bass.)
    assert np.sqrt(np.mean(difference**2)) > 0.002
    after = echo.audio[:, last + SR :] * scale - cut.audio[:, last + SR :]
    assert np.max(np.abs(after)) < 1e-4  # and they stop after 2 bars
