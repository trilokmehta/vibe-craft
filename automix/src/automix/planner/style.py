"""Transition styles and automatic style choice (Milestone 7).

Four styles, the same ones a DJ uses most:

    blend   Long beat-matched mix with EQ and bass swap. Sounds smoothest,
            but both tracks play together for a long time.
    filter  Beat-matched, but a high-pass filter slowly removes the body of
            track A (first the bass, then the mids), so B has room.
    cut     A stops on a phrase line and B starts on its "one". No overlap.
    echo    Like a cut, but the last beat of A repeats and fades away
            (echo) over the first bars of B. Hides a change of tempo.

How "auto" decides:

    1. Tempos more than 8 % apart            -> echo   (cannot beat-match)
    2. One track is calm during the overlap  -> blend  (nothing to fight)
    3. Both busy and the keys clash          -> cut    (overlap would sound sour)
    4. Both busy, keys fit                   -> filter (thin out A to make room)
"""

from __future__ import annotations

import numpy as np

from ..camelot import compatibility
from .simple import NoOverlapError, TransitionPlan, _active_bar_range, plan_simple_transition
from .vocals import add_vocal_plan
from .smart import FLOW_BARS, LOST_BARS_LIMIT, _mean, _phrase_lines, bar_levels, plan_smart_transition

STYLES = ("blend", "filter", "cut", "echo")
# Below this "clean" score both tracks count as busy during the overlap.
BUSY_BELOW = 0.5
# Below this key score an overlap is likely to sound sour.
KEY_CLASH_BELOW = 0.5


def choose_style(blend_plan: TransitionPlan) -> tuple[str, str]:
    """Pick a style for a planned blend. Returns (style, reason)."""
    if not blend_plan.beat_matched:
        return "echo", "tempos are more than 8 % apart, so the beats cannot be matched"
    clean = blend_plan.score_parts.get("clean")
    if clean is None or clean >= BUSY_BELOW:
        return "blend", "one track is calm during the overlap"
    if blend_plan.key_a and blend_plan.key_score < KEY_CLASH_BELOW:
        return "cut", "both tracks are busy and their keys clash"
    return "filter", "both tracks are busy, so the filter makes room for B"


def pick_cut_points(a: dict, b: dict, min_bar_a: int = 0) -> tuple[int, int]:
    """Bars for a cut or echo: (bar line of A where A stops, bar of B where B starts).

    There is no overlap, so only two things matter: the energy should not
    drop or jump (flow), and little music should be thrown away (keep).
    """
    levels_a, levels_b = bar_levels(a), bar_levels(b)
    lines_a, lines_b = _phrase_lines(a), _phrase_lines(b)
    range_a, range_b = _active_bar_range(a), _active_bar_range(b)

    half_a = range_a[0] + (range_a[1] - range_a[0]) * 0.5
    early_b = range_b[0] + (range_b[1] - range_b[0]) * 0.4
    cuts = sorted(s for s in lines_a | {range_a[1]} if max(half_a, min_bar_a) <= s <= range_a[1])
    if not cuts:  # nothing left after min_bar_a: stop A as late as it allows
        cuts = [max(range_a[1], min_bar_a)]
    starts = sorted({s for s in lines_b if range_b[0] <= s <= early_b} | {range_b[0]})

    best, best_score = (range_a[1], range_b[0]), -1.0
    for cut in cuts:
        for start in starts:
            before = _mean(levels_a, cut - FLOW_BARS, cut)
            after = _mean(levels_b, start, start + FLOW_BARS)
            change = after - before
            flow = 1.0 - (abs(change) if change >= 0 else 1.5 * abs(change))
            lost = max(range_a[1] - cut, 0) + (start - range_b[0])
            keep = 1.0 - min(1.0, lost / LOST_BARS_LIMIT)
            score = 0.5 * float(np.clip(flow, 0.0, 1.0)) + 0.5 * keep
            if score > best_score:
                best, best_score = (cut, start), score
    return best


def build_jump_plan(a: dict, b: dict, cut_bar: int, start_bar: int, style: str) -> TransitionPlan:
    """Plan for a cut or echo: A stops at bar line `cut_bar`, B starts at `start_bar`."""
    da, db = a["downbeats"], b["downbeats"]
    cut_bar = int(np.clip(cut_bar, 1, len(da) - 1))
    start_bar = int(np.clip(start_bar, 0, len(db) - 1))
    t_a, t_b = float(da[cut_bar]), float(db[start_bar])
    key_a = a.get("key", {}).get("camelot", "")
    key_b = b.get("key", {}).get("camelot", "")
    return TransitionPlan(
        a_start=t_a,
        a_end=t_a,
        b_start=t_b,
        b_end=t_b,
        bars_a=0,
        bars_b=0,
        stretch_rate=1.0,
        beat_matched=False,
        bpm_a=float(a["bpm"]),
        bpm_b=float(b["bpm"]),
        a_anchors=[t_a],
        b_anchors=[t_b],
        out_anchors=[0.0],
        bar_a=cut_bar,
        bar_b=start_bar,
        key_a=key_a,
        key_b=key_b,
        key_b_played=key_b,
        key_score=compatibility(key_a, key_b) if key_a and key_b else 0.0,
        key_note="no overlap, so no pitch shift",
        style=style,
        echo_beat=float(da[cut_bar] - da[cut_bar - 1]) / 4.0,
    )


def plan_transition(
    a: dict,
    b: dict,
    bars: int = 16,
    key_match: bool = True,
    style: str = "auto",
    smart: bool = True,
    min_bar_a: int = 0,
) -> TransitionPlan:
    """Plan a transition from analysis `a` to analysis `b`.

    style: "auto" or one of STYLES.
    smart: False uses the simple rule for the points (end of A, start of B).
    min_bar_a: the transition may not start before this bar of A. When A has
    no room left for an overlap, a cut is used (playlist mode).
    """
    if style != "auto" and style not in STYLES:
        raise ValueError(f"Unknown style: {style}")

    planner = plan_smart_transition if smart else plan_simple_transition
    try:
        plan = planner(a, b, bars=bars, key_match=key_match, min_bar_a=min_bar_a)
    except NoOverlapError:
        plan = None

    if plan is None:
        reason = "chosen by you"
        if style in ("auto", "blend", "filter"):
            style, reason = "cut", "not enough music left in the first track for an overlap"
    elif style == "auto":
        style, reason = choose_style(plan)
    else:
        reason = "chosen by you"

    if style in ("cut", "echo"):
        cut_bar, start_bar = pick_cut_points(a, b, min_bar_a)
        plan = build_jump_plan(a, b, cut_bar, start_bar, style)
    else:
        add_vocal_plan(plan, a, b)
    plan.style = style
    plan.style_reason = reason
    return plan
