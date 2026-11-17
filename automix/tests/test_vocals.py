"""Vocal-aware mixing tests. Demucs itself is not run here (too slow);
the "voices" are plain tones, so we can find them in the output."""

import numpy as np
import pytest
from synth import SR, make_track
from test_smart import LOUD, fake

from automix.analyzer.stems import bar_activity
from automix.planner.style import plan_transition
from automix.planner.vocals import pick_vocal_swap, vocal_clash
from automix.renderer.crossfade import render_transition
from automix.renderer.eq import vocal_gates


def with_vocals(analysis: dict, per_block: list[float], bars_each: int = 8) -> dict:
    analysis["vocals"] = {"bar_activity": [float(v) for v in np.repeat(per_block, bars_each)]}
    return analysis


def test_bar_activity():
    downbeats = [0.0, 1.0, 2.0, 3.0, 4.0]
    t = np.arange(4 * SR) / SR
    mix = 0.5 * np.sin(2 * np.pi * 200 * t)[np.newaxis, :]
    voice = 0.2 * np.sin(2 * np.pi * 800 * t)[np.newaxis, :]
    voice[:, : 2 * SR] = 0.0  # silent in bars 0 and 1
    activity = bar_activity(voice, mix, SR, downbeats)
    assert list(activity) == [0.0, 0.0, 1.0, 1.0]
    # A very faint "voice" (separation leftovers) means: instrumental track.
    assert not bar_activity(voice * 0.01, mix, SR, downbeats).any()


def test_vocal_clash():
    a, b = np.ones(16), np.concatenate([np.zeros(8), np.ones(8)])
    assert vocal_clash(a, b, 0, 0, 16) == 0.5
    assert vocal_clash(a, np.zeros(16), 0, 0, 16) == 0.0


def test_vocal_swap_prefers_a_pause():
    # A sings in the first 6 bars of the overlap, B from bar 6 on: swap at bar 6.
    a = np.array([1.0] * 6 + [0.0] * 10)
    b = np.array([0.0] * 6 + [1.0] * 10)
    assert pick_vocal_swap(a, b, 0, 0, 16) == 6
    # Both sing all the time: swap in the middle.
    assert pick_vocal_swap(np.ones(16), np.ones(16), 0, 0, 16) == 8


def test_planner_avoids_two_singers():
    # Without vocal data the planner uses the last bars of A and bar 0 of B.
    a, b = fake([LOUD] * 8), fake([LOUD] * 6)
    plain = plan_transition(a, b, style="blend")
    assert plain.vocal_clash is None and plain.bar_b == 0

    # B sings in its first 8 bars, then pauses for 8 bars. A sings all the
    # time. Entering B at bar 8 avoids two singers.
    a = with_vocals(fake([LOUD] * 8), [1.0] * 8)
    b = with_vocals(fake([LOUD] * 6), [1.0, 0.0, 1.0, 1.0, 1.0, 1.0])
    plan = plan_transition(a, b, style="blend")
    assert plan.bar_b == 8
    assert plan.vocal_clash == 0.0 and plan.vocal_swap is None
    assert plan.score_parts["vocal"] == 1.0


def test_vocal_gates_never_overlap():
    gate_a, gate_b = vocal_gates(10 * SR, SR, 5 * SR)
    assert gate_a[0] == 1.0 and gate_a[5 * SR] == 0.0 and gate_b[5 * SR - 1] == 0.0
    assert gate_b[-1] == 1.0
    assert np.max(np.minimum(gate_a, gate_b)) == 0.0


def _tone_level(audio: np.ndarray, hz: float, first: float, last: float) -> float:
    part = audio[0, int(first * SR) : int(last * SR)]
    spectrum = np.abs(np.fft.rfft(part * np.hanning(len(part)))) / len(part)
    return float(spectrum[int(round(hz * len(part) / SR))])


@pytest.mark.parametrize("style", ["blend", "filter"])
def test_only_one_voice_is_heard(style):
    # Two tracks at the same tempo. Their "voices" are tones: A 1000 Hz, B 1500 Hz.
    bars, bar = 32, 2.0
    n = int(bars * bar * SR)
    t = np.arange(n) / SR
    backing = make_track(120.0, bars)[np.newaxis, :n]
    voice_a = (0.2 * np.sin(2 * np.pi * 1000 * t)).astype(np.float32)[np.newaxis, :]
    voice_b = (0.2 * np.sin(2 * np.pi * 1500 * t)).astype(np.float32)[np.newaxis, :]

    a = with_vocals(fake([LOUD] * 4), [1.0] * 4)
    b = with_vocals(fake([LOUD] * 4), [1.0] * 4)
    plan = plan_transition(a, b, style=style, bars=8)
    assert plan.vocal_clash == 1.0 and plan.vocal_swap is not None

    result = render_transition(
        backing + voice_a, SR, backing + voice_b, SR, plan, vocals_a=voice_a, vocals_b=voice_b
    )
    start, swap, end = result.transition_start, result.vocal_swap, result.transition_end
    assert start < swap < end
    out = result.audio

    # Before the transition: A's voice only. After it: B's voice only.
    assert _tone_level(out, 1000, start - 3, start - 1) > 0.03
    assert _tone_level(out, 1500, end + 1, end + 3) > 0.03
    # Inside, before the swap: A's voice is there, B's is not.
    assert _tone_level(out, 1000, start + 1, swap - 1) > 0.01
    assert _tone_level(out, 1500, start + 1, swap - 1) < 0.002
    # Inside, after the swap: B's voice is there, A's is not.
    assert _tone_level(out, 1500, swap + 1, end - 1) > 0.01
    assert _tone_level(out, 1000, swap + 1, end - 1) < 0.002


def test_without_stems_nothing_changes():
    bars, n = 32, int(32 * 2.0 * SR)
    track = make_track(120.0, bars)[np.newaxis, :n]
    plan = plan_transition(fake([LOUD] * 4), fake([LOUD] * 4), style="blend", bars=8)
    result = render_transition(track, SR, track, SR, plan)
    assert result.vocal_swap is None and plan.vocal_clash is None
