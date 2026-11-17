"""Simple transition plan (Milestones 3 and 4).

Rule: leave track A during its last 16 bars (starting on a phrase line), and
bring in track B from its first full phrase.

Tempo ramp: the transition starts at A's tempo and ends at B's tempo. The
speed changes a little on every bar. Both tracks follow the same speed, so
their bar lines always fall together. After the transition B plays at its own
tempo, untouched.

The plan stores this as three lists of "anchors", one entry per bar line:
bar line k is at `a_anchors[k]` in A, at `b_anchors[k]` in B, and must be
played at `out_anchors[k]` seconds after the transition starts.

Smarter choices (energy, key, vocals) come in Milestone 6.
"""

from __future__ import annotations

from dataclasses import asdict, dataclass, field

import numpy as np

from ..camelot import best_shift, shift_code

# A DJ pitch fader usually goes to +-8 %. More than that sounds unnatural.
MAX_TEMPO_CHANGE = 0.08
# Keys are only trusted (and B only pitch-shifted) above this confidence.
MIN_KEY_CONFIDENCE = 0.1
# Phrases this much quieter than the loudest phrase count as silence.
SILENCE_DB = 15.0


class NoOverlapError(ValueError):
    """There is no room for an overlapping transition (the first track is too short)."""


@dataclass
class TransitionPlan:
    a_start: float  # seconds in track A where the transition starts
    a_end: float  # seconds in track A where it ends (A is silent after this)
    b_start: float  # seconds in track B (original time) where B starts playing
    b_end: float  # seconds in track B (original time) at the end of the transition
    bars_a: int  # transition length in bars of A
    bars_b: int  # bars of B played in that time (differs for half/double tempo)
    stretch_rate: float  # speed factor for B at the START. 1.05 = 5 % faster. Ends at 1.0
    beat_matched: bool  # False when the tempos are too far apart
    bpm_a: float
    bpm_b: float
    a_anchors: list[float] = field(default_factory=list)  # bar lines in A (seconds)
    b_anchors: list[float] = field(default_factory=list)  # the same bar lines in B
    out_anchors: list[float] = field(default_factory=list)  # seconds from transition start
    bass_swap: float = 0.0  # seconds from transition start (a bar line in the middle)
    steady_a: bool = True  # False when A's bar lines are uneven in the transition
    steady_b: bool = True
    key_a: str = ""  # Camelot code of A
    key_b: str = ""  # Camelot code of B as recorded
    pitch_shift_b: int = 0  # semitones B is shifted (whole rest of the track)
    key_b_played: str = ""  # Camelot code of B after the shift
    key_score: float = 0.0  # compatibility after the shift, 0.1 .. 1.0
    key_note: str = ""  # why B was or was not shifted
    bar_a: int = 0  # bar of A where the transition starts
    bar_b: int = 0  # bar of B where B starts playing
    score: float | None = None  # total score from the smart planner
    score_parts: dict = field(default_factory=dict)
    alternatives: list = field(default_factory=list)  # other good candidates
    style: str = "blend"  # blend, filter, cut or echo (see planner/style.py)
    style_reason: str = ""
    echo_beat: float = 0.0  # seconds per beat of A at the cut (echo style)
    vocal_clash: float | None = None  # share of the overlap where both sing (None = unknown)
    vocal_swap: float | None = None  # seconds from transition start where the voice changes A -> B

    def to_dict(self) -> dict:
        return asdict(self)


def _active_bar_range(analysis: dict) -> tuple[int, int]:
    """First and last bar (end exclusive) that are not silence."""
    phrases = analysis["phrases"]
    n_bars = len(analysis["downbeats"]) - 1
    if not phrases:
        return 0, n_bars
    loudest = max(p["energy_db"] for p in phrases)
    active = [p for p in phrases if p["energy_db"] > loudest - SILENCE_DB]
    first, last = active[0], active[-1]
    return first["start_bar"], last["start_bar"] + last["bars"]


def _mix_out_bar(analysis: dict, bars: int) -> int:
    """Bar of A where the transition starts: the last phrase line that still
    has `bars` bars of music after it."""
    first_bar, end_bar = _active_bar_range(analysis)
    latest = end_bar - bars
    starts = [p["start_bar"] for p in analysis["phrases"] if first_bar <= p["start_bar"] <= latest]
    return max(starts) if starts else max(first_bar, latest)


def _mix_in_bar(analysis: dict) -> int:
    """Bar of B where it starts playing: the first full, non-silent phrase."""
    first_bar, _ = _active_bar_range(analysis)
    for p in analysis["phrases"]:
        if p["start_bar"] >= first_bar and p["bars"] >= 8:
            return p["start_bar"]
    return first_bar


def _steady_grid(anchors: np.ndarray, tolerance: float = 0.04) -> tuple[np.ndarray, bool]:
    """Remove timing noise from a list of bar lines.

    The beat tracker places each bar line a few milliseconds early or late,
    and now and then it puts one clearly in the wrong place. Most recorded
    music has a fixed tempo, so the true bar lines are evenly spaced.

    We look for the evenly spaced grid that most bar lines agree with:
    start from the median bar length, then fit a straight line through the
    bar lines that are close to the grid (the wrong ones are left out).
    If fewer than half of the bar lines agree (live drummer, tempo change),
    the measured bar lines are kept as they are.

    Returns (bar lines, True when a steady grid was found).
    """
    index = np.arange(len(anchors), dtype=float)
    slope = float(np.median(np.diff(anchors)))
    offset = float(np.median(anchors - slope * index))
    for _ in range(3):
        error = np.abs(anchors - (slope * index + offset))
        good = error <= tolerance
        if good.sum() < max(3, len(anchors) // 2):
            return anchors, False
        slope, offset = np.polyfit(index[good], anchors[good], 1)
    return slope * index + offset, True


def _plan_key(a: dict, b: dict, key_match: bool) -> dict:
    """Decide whether to pitch-shift B so that its key fits A."""
    key_a, key_b = a.get("key"), b.get("key")
    if not key_a or not key_b:
        return {}
    code_a, code_b = key_a["camelot"], key_b["camelot"]
    shift, plain, shifted = best_shift(code_a, code_b)
    sure = min(key_a["confidence"], key_b["confidence"]) >= MIN_KEY_CONFIDENCE

    if shift == 0:
        note = "keys fit, no shift needed" if plain >= 0.6 else "no shift within 2 semitones helps"
    elif not key_match:
        shift, shifted, note = 0, plain, "key shift switched off"
    elif not sure:
        shift, shifted, note = 0, plain, "key detection not sure enough to shift"
    else:
        note = f"B shifted {shift:+d} semitones"
    return {
        "key_a": code_a,
        "key_b": code_b,
        "pitch_shift_b": shift,
        "key_b_played": shift_code(code_b, shift),
        "key_score": shifted,
        "key_note": note,
    }


def plan_simple_transition(
    a: dict, b: dict, bars: int = 16, key_match: bool = True, min_bar_a: int = 0
) -> TransitionPlan:
    """Plan one transition from analysis dict `a` to analysis dict `b`."""
    da, db = a["downbeats"], b["downbeats"]
    n_a, n_b = len(da) - 1, len(db) - 1
    if n_a < 4 or n_b < 4:
        raise ValueError("Not enough bars found to plan a transition.")

    bars = min(bars, n_a)
    i = min(_mix_out_bar(a, bars), n_a - bars)
    if i < min_bar_a:
        raise NoOverlapError(f"the transition must start at bar {min_bar_a} or later")
    return build_plan(a, b, i, _mix_in_bar(b), bars, key_match)


def build_plan(
    a: dict, b: dict, i: int, j: int, bars: int, key_match: bool = True
) -> TransitionPlan:
    """Make the full plan for a transition that starts at bar `i` of A and
    bar `j` of B and lasts `bars` bars."""
    da, db = a["downbeats"], b["downbeats"]
    n_b = len(db) - 1
    a_start, a_end = da[i], da[i + bars]
    length_a = a_end - a_start

    # Try the same number of bars first, then half and double. That handles
    # pairs like 70 and 140 BPM, where 8 bars of one last as long as 16 of the other.
    chosen = None
    for bars_b in (bars, bars // 2, bars * 2):
        if bars_b < 2 or j + bars_b > n_b:
            continue
        rate = (db[j + bars_b] - db[j]) / length_a
        if abs(rate - 1.0) <= MAX_TEMPO_CHANGE:
            chosen = (bars_b, rate)
            break

    if chosen:
        bars_b, rate = chosen
        b_end = db[j + bars_b]
        beat_matched = True
    else:
        # Tempos too far apart: plain crossfade, B at its own speed.
        bars_b, rate, beat_matched = bars, 1.0, False
        b_end = db[j] + length_a

    # One anchor per bar of A. B's anchors are spread over bars_b bars, so
    # with half/double tempo they fall on every 2nd bar line or mid-bar.
    a_anchors, steady_a = _steady_grid(np.array(da[i : i + bars + 1], dtype=float))
    steady_b = True
    bar_len_a = np.diff(a_anchors)
    if beat_matched:
        positions = j + np.arange(bars + 1) * (bars_b / bars)
        b_anchors, steady_b = _steady_grid(np.interp(positions, np.arange(len(db)), db))
        bar_len_b = np.diff(b_anchors)
        # Tempo ramp: early bars last as long as A's bars, late bars as long as B's.
        blend = (np.arange(bars) + 0.5) / bars
        bar_len_out = (1.0 - blend) * bar_len_a + blend * bar_len_b
    else:
        bar_len_out = bar_len_a
    out_anchors = np.concatenate([[0.0], np.cumsum(bar_len_out)])
    if not beat_matched:
        b_anchors = db[j] + out_anchors

    return TransitionPlan(
        a_start=float(a_anchors[0]),
        a_end=float(a_anchors[-1]),
        b_start=float(b_anchors[0]),
        b_end=float(b_anchors[-1]),
        bars_a=bars,
        bars_b=bars_b,
        stretch_rate=float(rate),
        beat_matched=beat_matched,
        bpm_a=float(a["bpm"]),
        bpm_b=float(b["bpm"]),
        a_anchors=[float(v) for v in a_anchors],
        b_anchors=[float(v) for v in b_anchors],
        out_anchors=[float(v) for v in out_anchors],
        bass_swap=float(out_anchors[bars // 2]),
        steady_a=bool(steady_a),
        steady_b=bool(steady_b),
        bar_a=int(i),
        bar_b=int(j),
        **_plan_key(a, b, key_match),
    )
