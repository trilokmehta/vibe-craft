"""Vocal rules for transitions (Milestone 8).

Two singers at the same time is the most obvious mistake a mix can have.
Two things are done about it:

1. Planning: transitions where both tracks sing during the overlap get a
   lower score, so the planner prefers "A sings over B's intro" or
   "B sings over A's outro".

2. Vocal swap: if both still sing in the overlap, only one voice is heard at
   a time. A's voice plays until a bar line near the middle, then B's voice
   takes over. The bar line is chosen where the voices pause, if they do.

Each analysis may carry a "vocals" entry with "bar_activity": one number per
bar, 0 = no singing, 1 = full singing (see analyzer/stems.py).
"""

from __future__ import annotations

import numpy as np

# Both singing in less than this share of the overlap: leave the audio alone.
MIN_CLASH = 0.05


def activity(analysis: dict) -> np.ndarray | None:
    """Vocal activity per bar, or None when the track has no vocal analysis."""
    vocals = analysis.get("vocals")
    if not vocals or "bar_activity" not in vocals:
        return None
    return np.asarray(vocals["bar_activity"], dtype=float)


def _overlap(act_a, act_b, i: int, j: int, bars: int):
    over_a, over_b = act_a[i : i + bars], act_b[j : j + bars]
    n = min(len(over_a), len(over_b))
    return over_a[:n], over_b[:n]


def vocal_clash(act_a, act_b, i: int, j: int, bars: int) -> float:
    """Share of the overlap in which both tracks sing. 0 = never, 1 = always."""
    over_a, over_b = _overlap(act_a, act_b, i, j, bars)
    return float(np.minimum(over_a, over_b).mean()) if len(over_a) else 0.0


def pick_vocal_swap(act_a, act_b, i: int, j: int, bars: int) -> int:
    """Bar line (1 .. bars-1, counted from the transition start) where the
    voice changes from A to B.

    Looks in the middle half of the transition for the bar line where the
    least singing is cut: A's voice is cut after the line, B's before it.
    """
    over_a, over_b = _overlap(act_a, act_b, i, j, bars)
    n = len(over_a)
    middle = n // 2
    if n < 4:
        return max(1, middle)
    best, best_cost = middle, float("inf")
    for k in range(max(1, n // 4), min(n - 1, 3 * n // 4) + 1):
        cut_singing = float(over_a[k:].sum() + over_b[:k].sum())
        # Small extra cost for cutting in the middle of a sung line, and
        # for moving away from the middle.
        mid_line = float(over_a[k - 1] * over_a[k] + over_b[k - 1] * over_b[k])
        cost = cut_singing + 0.5 * mid_line + 0.05 * abs(k - middle)
        if cost < best_cost:
            best, best_cost = k, cost
    return best


def add_vocal_plan(plan, a: dict, b: dict) -> None:
    """Fill plan.vocal_clash and plan.vocal_swap for a blend or filter plan."""
    act_a, act_b = activity(a), activity(b)
    if act_a is None or act_b is None or plan.bars_a < 2:
        return
    # With half/double tempo B plays a different number of bars; compare
    # A's bars with the B bars that sound at the same time.
    bars = plan.bars_a
    if plan.bars_b != bars:
        positions = plan.bar_b + np.arange(bars) * (plan.bars_b / bars)
        act_b = np.concatenate([np.zeros(plan.bar_b), act_b[np.clip(positions.astype(int), 0, len(act_b) - 1)]])
    plan.vocal_clash = vocal_clash(act_a, act_b, plan.bar_a, plan.bar_b, bars)
    if plan.vocal_clash >= MIN_CLASH:
        k = pick_vocal_swap(act_a, act_b, plan.bar_a, plan.bar_b, bars)
        plan.vocal_swap = float(plan.out_anchors[k])
