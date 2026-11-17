"""Smart planner tests with hand-made analysis data (no audio needed)."""

import numpy as np

from automix.planner.smart import bar_levels, plan_smart_transition, rank_candidates

BAR = 2.0  # seconds per bar at 120 BPM
STEP = 0.5  # seconds per energy value


def fake(
    levels_db: list[float],
    bars_each: int = 8,
    bar: float = BAR,
    key: str = "8A",
    confidence: float = 0.5,
) -> dict:
    """A track made of blocks of `bars_each` bars with the given loudness."""
    n_bars = len(levels_db) * bars_each
    phrases = [
        {"index": k, "start_bar": k * bars_each, "bars": bars_each, "energy_db": db, "label": "x"}
        for k, db in enumerate(levels_db)
    ]
    per_bar = np.repeat(levels_db, bars_each)
    rms = np.repeat(per_bar, 4)
    return {
        "bpm": 240.0 / bar,
        "downbeats": [k * bar for k in range(n_bars + 1)],
        "phrases": phrases,
        "energy": {"step_seconds": bar / 4, "rms_db": [float(v) for v in rms]},
        "key": {"camelot": key, "confidence": confidence},
    }


LOUD, MID, QUIET = -10.0, -14.0, -22.0


def test_bar_levels():
    levels = bar_levels(fake([QUIET, LOUD, LOUD, MID]))
    assert len(levels) == 32
    assert levels[0] == 0.0 and levels[8] == 1.0
    assert 0.5 < levels[24] < 0.8


def test_candidates_are_sorted_and_in_range():
    a, b = fake([LOUD] * 8), fake([QUIET, LOUD, LOUD, LOUD, LOUD, LOUD])
    found = rank_candidates(a, b)
    assert found and all(x["score"] >= y["score"] for x, y in zip(found, found[1:]))
    for c in found:
        assert all(0.0 <= v <= 1.0 for v in c["parts"].values())
        assert c["i"] % 8 == 0 and c["j"] % 8 == 0  # phrase lines only


def test_uses_the_quiet_intro_of_b():
    # A is loud to the end. B has a 16-bar quiet intro, then it is loud.
    a, b = fake([LOUD] * 8), fake([QUIET, QUIET, LOUD, LOUD, LOUD, LOUD])
    plan = plan_smart_transition(a, b)
    assert plan.bar_b == 0 and plan.bars_a == 16  # long blend over the intro
    assert plan.bar_a == 48  # the last 16 bars of A
    assert plan.score_parts["clean"] == 1.0
    assert plan.alternatives[0]["score"] == plan.score


def test_short_transition_when_both_are_busy():
    # Both tracks are at full power everywhere: keep the overlap short.
    a, b = fake([LOUD] * 8), fake([LOUD] * 6)
    plan = plan_smart_transition(a, b)
    assert plan.bars_a == 8
    assert plan.bar_a == 56 and plan.bar_b == 0


def test_leaves_a_before_its_quiet_ending():
    # A ends with 8 quiet bars (not silent). B starts loud right away.
    # Fading from A's quiet ending into loud B is a big energy jump, but
    # mixing over the quiet ending is clean. Both points must be phrase lines.
    a, b = fake([LOUD] * 7 + [MID]), fake([LOUD] * 6)
    plan = plan_smart_transition(a, b)
    assert plan.bar_a in (48, 56) and plan.bar_b == 0
    assert abs(plan.a_start - plan.bar_a * BAR) < 1e-6


def test_falls_back_for_very_short_tracks():
    a, b = fake([LOUD]), fake([LOUD])
    plan = plan_smart_transition(a, b)
    assert plan.score is None  # simple rule was used
    assert plan.bars_a <= 8
