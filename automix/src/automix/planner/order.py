"""Track order for a playlist (optional, `--order smart`).

Greedy: start with the first track, then always play next the unused track
that is easiest to mix into the current one. "Easy" means:

    tempo  within 8 % (also counting half and double time): the beats can be
           matched. 1.0 for equal tempo, 0.0 at 8 % or more apart.
    key    Camelot compatibility of the two keys (see camelot.py).

A greedy order is not the best possible order, but it is fast, easy to
understand, and a good start. The first track is always the first in the list.
"""

from __future__ import annotations

from ..camelot import compatibility
from .simple import MAX_TEMPO_CHANGE


def pair_score(a: dict, b: dict) -> float:
    """How easy it is to mix track `b` after track `a`. 0 = hard, 1 = easy."""
    key_a, key_b = a.get("key"), b.get("key")
    key = compatibility(key_a["camelot"], key_b["camelot"]) if key_a and key_b else 0.5

    bpm_a, bpm_b = float(a["bpm"]), float(b["bpm"])
    if bpm_a <= 0 or bpm_b <= 0:
        tempo = 0.0
    else:
        ratio = bpm_b / bpm_a
        off = min(abs(ratio * 2.0**k - 1.0) for k in (-1, 0, 1))
        tempo = max(0.0, 1.0 - off / MAX_TEMPO_CHANGE)
    return 0.5 * tempo + 0.5 * key


def order_tracks(analyses: list[dict]) -> list[int]:
    """Indices of `analyses` in the order they should be played."""
    if not analyses:
        return []
    order = [0]
    remaining = list(range(1, len(analyses)))
    while remaining:
        current = analyses[order[-1]]
        best = max(remaining, key=lambda i: (pair_score(current, analyses[i]), -i))
        order.append(best)
        remaining.remove(best)
    return order
