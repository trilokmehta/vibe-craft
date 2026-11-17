"""Camelot wheel: which keys sound good together ("harmonic mixing").

A Camelot code is a number 1-12 plus a letter: A = minor, B = major.
The numbers are arranged like a clock. One step on the clock is a "perfect
fifth" (7 semitones), and keys one step apart share 6 of their 7 notes.

Rules DJs use (from track A to track B):
    same code (8A -> 8A)                  perfect
    same number, other letter (8A -> 8B)  relative major/minor, same notes
    one step, same letter (8A -> 7A, 9A)  6 of 7 notes shared
    one step, other letter (8A -> 9B)     "diagonal", usually fine
    two steps up (8A -> 10A)              "energy boost", whole tone up
    seven steps up (8A -> 3A)             one semitone up, a classic key lift
    everything else                       likely to clash
"""

from __future__ import annotations

# Pitch shifts larger than this start to sound unnatural (voices especially).
MAX_SHIFT = 2
# Each semitone of shift costs this much score, so we shift only when it helps.
SHIFT_PENALTY = 0.05
# Shift only when the score gets better by at least this much.
MIN_IMPROVEMENT = 0.2


def camelot_code(pitch_class: int, mode: str) -> str:
    """Convert a key to Camelot notation. pitch_class: C=0, C#=1, ... B=11.

    Going up a perfect fifth (7 semitones) adds 1 to the Camelot number.
    C major is 8B and A minor is 8A.
    """
    fifths_from_c = (7 * pitch_class) % 12
    if mode == "major":
        return f"{(fifths_from_c + 7) % 12 + 1}B"
    return f"{(fifths_from_c + 4) % 12 + 1}A"


def parse(code: str) -> tuple[int, str]:
    """ "10B" -> (10, "B")"""
    return int(code[:-1]), code[-1].upper()


def shift_code(code: str, semitones: int) -> str:
    """Camelot code after pitch-shifting by `semitones`.

    One semitone up moves 7 steps around the wheel. The letter stays.
    """
    number, letter = parse(code)
    return f"{(number - 1 + 7 * semitones) % 12 + 1}{letter}"


def compatibility(code_a: str, code_b: str) -> float:
    """How well track B's key follows track A's key. 1.0 = perfect, 0.1 = clash."""
    num_a, let_a = parse(code_a)
    num_b, let_b = parse(code_b)
    steps = (num_b - num_a) % 12  # steps clockwise from A to B
    same_letter = let_a == let_b

    if steps == 0:
        return 1.0 if same_letter else 0.9
    if steps in (1, 11):
        return 0.9 if same_letter else 0.6
    if same_letter and steps == 2:
        return 0.5
    if same_letter and steps == 7:
        return 0.4
    return 0.1


def best_shift(code_a: str, code_b: str) -> tuple[int, float, float]:
    """Find the pitch shift for B (in semitones, -2..+2) that fits A best.

    Returns (shift, score without shift, score with that shift).
    The shift is 0 unless shifting makes the match clearly better.
    """
    plain = compatibility(code_a, code_b)
    best, best_value = 0, plain
    for shift in (1, -1, 2, -2):
        if abs(shift) > MAX_SHIFT:
            continue
        value = compatibility(code_a, shift_code(code_b, shift)) - SHIFT_PENALTY * abs(shift)
        if value > best_value + 1e-9:
            best, best_value = shift, value
    if best != 0 and best_value < plain + MIN_IMPROVEMENT:
        best = 0
    return best, plain, compatibility(code_a, shift_code(code_b, best))
