"""Smart transition points (Milestone 6).

The simple planner always leaves A at its very end and enters B at its very
start. This planner looks at many possible pairs of points and gives each a
score, like a DJ who thinks "where does this song calm down, and where does
the next one pick up?".

Candidates
    Mix-out: every phrase line in the second half of A.
    Mix-in:  every phrase line in the first 40 % of B.
    Length:  8, 16 or 32 bars (not longer than the --bars option).

Score parts (each 0 = bad, 1 = good)
    flow    Energy before the transition (A) and after it (B) should be
            similar. A drop in energy is punished more than a rise.
    clean   While both tracks play, they should not both be at full power.
            Best is A's outro over B's intro. Uses the energy per bar.
    keep    Little music should be thrown away (the end of A that is cut,
            the start of B that is skipped). 32 lost bars = 0.
    phrase  The four corner points (start and end of the transition, in A
            and in B) should be phrase lines.
    length  Small bonus for a long transition when it is clean, and for a
            short one when both tracks are busy.
    vocal   Only with vocal analysis (--vocals): 1 when the two tracks never
            sing at the same time during the overlap, 0 when they always do.

The weights below are a first guess. Change them and listen.
"""

from __future__ import annotations

import numpy as np

from .simple import (
    NoOverlapError,
    TransitionPlan,
    _active_bar_range,
    build_plan,
    plan_simple_transition,
)
from .vocals import activity, vocal_clash

WEIGHTS = {"flow": 0.25, "clean": 0.25, "keep": 0.30, "phrase": 0.20}
LENGTH_WEIGHT = 0.10
# When vocal data exists: total = (1 - VOCAL_WEIGHT) * the parts above + VOCAL_WEIGHT * vocal.
VOCAL_WEIGHT = 0.25
LENGTH_OPTIONS = (8, 16, 32)
# A bar this many dB below the loud parts of the track counts as "empty".
QUIET_DB = 12.0
LOST_BARS_LIMIT = 32
FLOW_BARS = 8  # bars before / after the transition that are compared


def bar_levels(analysis: dict) -> np.ndarray:
    """Energy of every bar on a scale from 0 (quiet) to 1 (as loud as the
    loud parts of this track)."""
    downbeats = np.asarray(analysis["downbeats"], dtype=float)
    n_bars = len(downbeats) - 1
    energy = analysis.get("energy")
    if not energy or not energy.get("rms_db"):
        # No energy curve: use the phrase energies.
        db = np.full(n_bars, -60.0)
        for p in analysis["phrases"]:
            db[p["start_bar"] : p["start_bar"] + p["bars"]] = p["energy_db"]
    else:
        step = energy["step_seconds"]
        power = 10.0 ** (np.asarray(energy["rms_db"], dtype=float) / 10.0)
        db = np.empty(n_bars)
        for k in range(n_bars):
            first = min(int(downbeats[k] / step), len(power) - 1)
            last = max(first + 1, int(np.ceil(downbeats[k + 1] / step)))
            db[k] = 10.0 * np.log10(max(float(power[first:last].mean()), 1e-12))
    loud = np.percentile(db, 90)
    return np.clip((db - loud + QUIET_DB) / QUIET_DB, 0.0, 1.0)


def _phrase_lines(analysis: dict) -> set[int]:
    lines = set()
    for p in analysis["phrases"]:
        lines.add(p["start_bar"])
        lines.add(p["start_bar"] + p["bars"])
    return lines


def _mean(levels: np.ndarray, first: int, last: int) -> float:
    part = levels[max(first, 0) : max(last, 0)]
    return float(part.mean()) if len(part) else 0.0


def score_candidate(
    levels_a: np.ndarray,
    levels_b: np.ndarray,
    lines_a: set[int],
    lines_b: set[int],
    range_a: tuple[int, int],
    range_b: tuple[int, int],
    i: int,
    j: int,
    bars: int,
    max_bars: int,
) -> tuple[float, dict]:
    """Score a transition that starts at bar i of A and bar j of B."""
    # flow: energy just before the transition vs. just after it.
    before = _mean(levels_a, i - FLOW_BARS, i)
    after = _mean(levels_b, j + bars, j + bars + FLOW_BARS)
    change = after - before
    flow = 1.0 - (abs(change) if change >= 0 else 1.5 * abs(change))

    # clean: in every bar of the overlap, the quieter track decides how
    # crowded it sounds. Two full tracks -> 1.0 crowded, one empty -> 0.0.
    over_a = levels_a[i : i + bars]
    over_b = levels_b[j : j + bars]
    n = min(len(over_a), len(over_b))
    crowded = float(np.minimum(over_a[:n], over_b[:n]).mean()) if n else 1.0
    clean = 1.0 - crowded

    # keep: bars of music that nobody will hear.
    lost = (range_a[1] - (i + bars)) + (j - range_b[0])
    keep = 1.0 - min(1.0, max(lost, 0) / LOST_BARS_LIMIT)

    # phrase: corner points on phrase lines.
    corners = [i in lines_a, i + bars in lines_a, j in lines_b, j + bars in lines_b]
    phrase = sum(corners) / 4.0

    parts = {
        "flow": float(np.clip(flow, 0.0, 1.0)),
        "clean": clean,
        "keep": keep,
        "phrase": phrase,
    }
    total = sum(WEIGHTS[name] * parts[name] for name in WEIGHTS)
    total += LENGTH_WEIGHT * (bars / max_bars) * (1.0 - 2.0 * crowded)
    return float(total), parts


def rank_candidates(a: dict, b: dict, bars: int = 16, min_bar_a: int = 0) -> list[dict]:
    """All candidates, best first. Each is a dict with i, j, bars, score, parts.

    min_bar_a: the transition may not start before this bar of A (playlist
    mode: the first part of A is already used by the transition before).
    """
    levels_a, levels_b = bar_levels(a), bar_levels(b)
    lines_a, lines_b = _phrase_lines(a), _phrase_lines(b)
    range_a, range_b = _active_bar_range(a), _active_bar_range(b)
    lengths = [n for n in LENGTH_OPTIONS if n <= bars] or [bars]
    vocals_a, vocals_b = activity(a), activity(b)

    half_a = range_a[0] + (range_a[1] - range_a[0]) * 0.5
    early_b = range_b[0] + (range_b[1] - range_b[0]) * 0.4
    starts_a = sorted(s for s in lines_a if half_a <= s < range_a[1] and s >= min_bar_a)
    starts_b = sorted({s for s in lines_b if range_b[0] <= s <= early_b} | {range_b[0]})

    found = []
    for length in lengths:
        for i in starts_a:
            if i + length > range_a[1]:
                continue
            for j in starts_b:
                # B must still have music left after the transition.
                if j + length + FLOW_BARS > range_b[1]:
                    continue
                total, parts = score_candidate(
                    levels_a, levels_b, lines_a, lines_b, range_a, range_b, i, j, length, max(lengths)
                )
                if vocals_a is not None and vocals_b is not None:
                    parts["vocal"] = 1.0 - vocal_clash(vocals_a, vocals_b, i, j, length)
                    total = (1.0 - VOCAL_WEIGHT) * total + VOCAL_WEIGHT * parts["vocal"]
                found.append({"i": i, "j": j, "bars": length, "score": total, "parts": parts})
    found.sort(key=lambda c: c["score"], reverse=True)
    return found


def plan_smart_transition(
    a: dict,
    b: dict,
    bars: int = 16,
    key_match: bool = True,
    keep_alternatives: int = 5,
    min_bar_a: int = 0,
) -> TransitionPlan:
    """Plan the best-scoring transition from analysis `a` to analysis `b`.

    `bars` is the longest transition allowed. Falls back to the simple rule
    when a track is too short to offer any candidate. With `min_bar_a` set and
    no candidate left, it raises NoOverlapError.
    """
    candidates = rank_candidates(a, b, bars, min_bar_a)
    if not candidates:
        if min_bar_a > 0:
            raise NoOverlapError(f"no transition fits after bar {min_bar_a}")
        return plan_simple_transition(a, b, bars=bars, key_match=key_match)

    best = candidates[0]
    plan = build_plan(a, b, best["i"], best["j"], best["bars"], key_match)
    plan.score = best["score"]
    plan.score_parts = best["parts"]
    da, db = a["downbeats"], b["downbeats"]
    plan.alternatives = [
        {**c, "a_start": float(da[c["i"]]), "b_start": float(db[c["j"]])}
        for c in candidates[:keep_alternatives]
    ]
    return plan
