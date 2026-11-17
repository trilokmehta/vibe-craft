"""Joins the three parts: analyze -> plan -> render -> write WAV."""

from __future__ import annotations

from pathlib import Path

import soundfile as sf

from .analyzer import DEFAULT_CACHE_DIR, analyze_track
from .audio_io import load_audio
from .planner.simple import TransitionPlan
from .planner.style import plan_transition
from .renderer.crossfade import RenderResult, render_transition


def mix_two_tracks(
    path_a: str | Path,
    path_b: str | Path,
    out_path: str | Path,
    bars: int = 16,
    preview_seconds: float | None = None,
    cache_dir: str | Path | None = DEFAULT_CACHE_DIR,
    beat_backend: str = "auto",
    bass_swap: bool = True,
    key_match: bool = True,
    smart: bool = True,
    style: str = "auto",
    vocals: bool = False,
    plot: str | Path | None = None,
) -> tuple[TransitionPlan, RenderResult]:
    """Mix track A into track B and write the result as a WAV file.

    preview_seconds: when set, only write this many seconds before and after
    the transition. Handy for checking a transition quickly.
    vocals: separate the vocals with Demucs (slow the first time per song),
    avoid two singers in the plan, and let only one voice sing at a time.
    plot: when set, write a debug picture of the transition (PNG) to this path.
    It is drawn from the full result, before any preview cut.
    """
    analysis_a = analyze_track(path_a, cache_dir=cache_dir, beat_backend=beat_backend)
    analysis_b = analyze_track(path_b, cache_dir=cache_dir, beat_backend=beat_backend)
    if vocals:
        from .analyzer.stems import vocal_analysis

        analysis_a["vocals"] = vocal_analysis(path_a, analysis_a)
        analysis_b["vocals"] = vocal_analysis(path_b, analysis_b)
    plan = plan_transition(
        analysis_a, analysis_b, bars=bars, key_match=key_match, style=style, smart=smart
    )

    audio_a, sr_a = load_audio(path_a)
    audio_b, sr_b = load_audio(path_b)
    stems = {}
    if vocals and plan.vocal_swap is not None:
        from .analyzer.stems import load_vocals

        stems["vocals_a"] = load_vocals(analysis_a["vocals"])[0]
        stems["vocals_b"] = load_vocals(analysis_b["vocals"])[0]
    result = render_transition(
        audio_a,
        sr_a,
        audio_b,
        sr_b,
        plan,
        lufs_a=analysis_a["loudness_lufs"],
        lufs_b=analysis_b["loudness_lufs"],
        bass_swap=bass_swap,
        **stems,
    )

    if plot is not None:
        from .debug_plot import plot_transition

        plot_transition(
            plot, plan, analysis_a, analysis_b, audio_a, sr_a, audio_b, sr_b, result
        )

    if preview_seconds is not None:
        first = max(0, int((result.transition_start - preview_seconds) * result.sr))
        last = int((result.transition_end + preview_seconds) * result.sr)
        result.audio = result.audio[:, first:last]
        result.transition_end -= first / result.sr
        result.transition_start -= first / result.sr
        if result.bass_swap is not None:
            result.bass_swap -= first / result.sr
        if result.vocal_swap is not None:
            result.vocal_swap -= first / result.sr

    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    sf.write(str(out_path), result.audio.T, result.sr, subtype="PCM_16")
    return plan, result
