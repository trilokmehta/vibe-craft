"""Debug plot of one transition (Milestone 10).

Three rows, one picture:

    track A    waveform with beats, bars, phrase lines, and the part used
    track B    the same for B (shown in B's original time)
    output     the finished mix with the transition window, bass swap and
               voice swap

Needs matplotlib (pip install matplotlib). It is imported only when a plot is
asked for, so the rest of automix works without it.
"""

from __future__ import annotations

from pathlib import Path

import numpy as np

ENVELOPE_POINTS = 2000  # a screen cannot show more than this


def envelope(audio: np.ndarray, sr: int, points: int = ENVELOPE_POINTS):
    """Peak envelope of (channels, samples) audio.

    Returns (times in seconds, upper, lower), each `points` long at most.
    """
    mono = np.asarray(audio, dtype=np.float32)
    mono = mono.mean(axis=0) if mono.ndim == 2 else mono
    n = len(mono)
    if n == 0:
        return np.zeros(0), np.zeros(0), np.zeros(0)
    points = max(1, min(points, n))
    edges = np.linspace(0, n, points + 1).astype(int)
    upper = np.empty(points, dtype=np.float32)
    lower = np.empty(points, dtype=np.float32)
    for i in range(points):
        chunk = mono[edges[i] : max(edges[i + 1], edges[i] + 1)]
        upper[i] = chunk.max()
        lower[i] = chunk.min()
    times = (edges[:-1] + edges[1:]) / 2.0 / sr
    return times, upper, lower


def _track_row(ax, audio, sr, analysis, title, used, used_color, offset=0.0):
    times, upper, lower = envelope(audio, sr)
    ax.fill_between(times + offset, lower, upper, color="#6b7280", linewidth=0)
    for beat in analysis.get("beats", []):
        ax.axvline(beat, color="#9ca3af", linewidth=0.3, alpha=0.5)
    for bar in analysis.get("downbeats", []):
        ax.axvline(bar, color="#2563eb", linewidth=0.5, alpha=0.6)
    for phrase in analysis.get("phrases", []):
        ax.axvline(phrase["start"], color="#dc2626", linewidth=1.2, alpha=0.8)
        ax.text(
            phrase["start"],
            1.0,
            f" {phrase['index']}",
            color="#dc2626",
            fontsize=7,
            va="top",
            transform=ax.get_xaxis_transform(),
        )
    ax.axvspan(used[0], used[1], color=used_color, alpha=0.25)
    ax.set_title(title, loc="left", fontsize=9)
    ax.set_ylim(-1.05, 1.05)
    ax.set_yticks([])


def plot_transition(
    out_path: str | Path,
    plan,
    analysis_a: dict,
    analysis_b: dict,
    audio_a: np.ndarray,
    sr_a: int,
    audio_b: np.ndarray,
    sr_b: int,
    result,
) -> Path:
    """Write a PNG that shows where and how the transition happens."""
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    fig, axes = plt.subplots(3, 1, figsize=(14, 8.5))

    name_a = Path(analysis_a.get("file", "A")).name
    name_b = Path(analysis_b.get("file", "B")).name
    _track_row(
        axes[0],
        audio_a,
        sr_a,
        analysis_a,
        f"A: {name_a}   {plan.bpm_a:.1f} BPM   key {plan.key_a or '?'}",
        (plan.a_start, plan.a_end),
        "#f59e0b",
    )
    _track_row(
        axes[1],
        audio_b,
        sr_b,
        analysis_b,
        f"B: {name_b}   {plan.bpm_b:.1f} BPM   key {plan.key_b or '?'}"
        + (f" (played as {plan.key_b_played})" if plan.pitch_shift_b else ""),
        (plan.b_start, plan.b_end),
        "#10b981",
    )
    axes[0].set_xlabel("seconds in A")
    axes[1].set_xlabel("seconds in B (original time)")

    # Output row.
    ax = axes[2]
    times, upper, lower = envelope(result.audio, result.sr)
    ax.fill_between(times, lower, upper, color="#374151", linewidth=0)
    ax.axvspan(result.transition_start, result.transition_end, color="#8b5cf6", alpha=0.22)
    for position, label, color in (
        (result.transition_start, "start", "#8b5cf6"),
        (result.transition_end, "end", "#8b5cf6"),
        (result.bass_swap, "bass swap", "#dc2626"),
        (result.vocal_swap, "voice swap", "#0891b2"),
    ):
        if position is None:
            continue
        ax.axvline(position, color=color, linewidth=1.4)
        ax.text(
            position,
            1.0,
            f" {label}",
            color=color,
            fontsize=8,
            va="top",
            transform=ax.get_xaxis_transform(),
        )
    ax.set_ylim(-1.05, 1.05)
    ax.set_yticks([])
    ax.set_xlabel("seconds in the output")
    ax.set_title(
        f"Output: {plan.style} transition ({plan.style_reason})"
        f"   score {'-' if plan.score is None else f'{plan.score:.2f}'}",
        loc="left",
        fontsize=9,
    )

    fig.tight_layout()
    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(out_path, dpi=110)
    plt.close(fig)
    return out_path
