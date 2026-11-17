"""Renders a planned transition between two tracks into one piece of audio.

Output = [A, untouched] + [transition] + [B, untouched]

What happens in the transition depends on the style of the plan:

    blend   both tracks follow the tempo ramp, each frequency band has its
            own fade, the bass swaps on a bar line
    filter  like blend, but A is thinned out by a rising high-pass filter
    cut     A stops, B starts (10 ms fade so there is no click)
    echo    like cut, plus the last beat of A repeating and fading over B
"""

from __future__ import annotations

from dataclasses import dataclass

import librosa
import numpy as np
from scipy.signal import butter, sosfiltfilt

from ..planner.simple import TransitionPlan
from .eq import BANDS, band_gains, equal_power_curves, split_bands, sweep_highpass, vocal_gates
from .limiter import limit
from .stretch import pitch_shift, stretch_to_map

__all__ = ["RenderResult", "render_transition", "equal_power_curves"]

# Extra audio stretched on the outside of the transition. It is blended with
# the untouched audio, so there is no click where the two meet.
SEAM = 0.25  # seconds
CUT_FADE = 0.01  # seconds
ECHO_REPEATS = 8  # one per beat = 2 bars
ECHO_FEEDBACK = 0.65  # each repeat is this much quieter than the one before
ECHO_LOW_CUT_HZ = 250.0  # echoes carry no bass, so they do not fight B's kick
LIMIT_CEILING = 0.98  # about -0.2 dBFS
FILTER_FADE_PART = 0.25  # A fades out during the last quarter of a filter transition


@dataclass
class RenderResult:
    audio: np.ndarray  # (channels, samples)
    sr: int
    transition_start: float  # seconds in the output
    transition_end: float
    bass_swap: float | None  # seconds in the output, None when there is no swap
    stretch_engine: str
    gain_b_db: float  # loudness change applied to track B
    master_gain_db: float  # change applied to everything to avoid clipping
    style: str = "blend"
    vocal_swap: float | None = None  # seconds in the output, None when not used
    # Playlist mode: B plays untouched from sample `b_resume` of the (processed)
    # track B. In `audio` that is the sample `out_resume`.
    out_resume: int = 0
    b_resume: int = 0


def _to_stereo(audio: np.ndarray) -> np.ndarray:
    if audio.shape[0] == 1:
        return np.repeat(audio, 2, axis=0)
    return audio[:2]


def _fit(audio: np.ndarray, n: int) -> np.ndarray:
    """Cut or zero-pad (channels, samples) audio to exactly n samples."""
    if audio.shape[1] >= n:
        return audio[:, :n]
    return np.pad(audio, ((0, 0), (0, n - audio.shape[1])))


def _stretch_a(a: np.ndarray, sr: int, plan: TransitionPlan) -> tuple[np.ndarray, str]:
    """Track A's part of the transition, following the tempo ramp."""
    seam = min(SEAM, plan.a_start)
    first = int(round((plan.a_start - seam) * sr))
    last = int(round(plan.a_end * sr))
    src = np.concatenate([[0.0], np.array(plan.a_anchors) - plan.a_start + seam])
    dst = np.concatenate([[0.0], np.array(plan.out_anchors) + seam])
    out, engine = stretch_to_map(a[:, first:last], sr, src, dst)

    # Blend from untouched A into stretched A during the seam.
    n_seam = int(round(seam * sr))
    if n_seam > 0 and engine != "none":
        w = np.linspace(0.0, 1.0, n_seam, dtype=np.float32)
        out[:, :n_seam] = a[:, first : first + n_seam] * (1.0 - w) + out[:, :n_seam] * w
    return out[:, n_seam:], engine


def _stretch_b(b: np.ndarray, sr: int, plan: TransitionPlan) -> tuple[np.ndarray, str, int]:
    """Track B's part of the transition (plus the seam), following the tempo ramp.

    Also returns the sample of B where the untouched rest continues.
    """
    first = int(round(plan.b_start * sr))
    last = min(int(round((plan.b_end + SEAM) * sr)), b.shape[1])
    seam = last / sr - plan.b_end
    src = np.concatenate([np.array(plan.b_anchors) - plan.b_start, [last / sr - plan.b_start]])
    dst = np.concatenate([plan.out_anchors, [plan.out_anchors[-1] + seam]])
    out, engine = stretch_to_map(b[:, first:last], sr, src, dst)

    # Blend from stretched B back into untouched B during the seam.
    n = int(round(plan.out_anchors[-1] * sr))
    n_seam = out.shape[1] - n
    if n_seam > 0 and engine != "none":
        w = np.linspace(0.0, 1.0, n_seam, dtype=np.float32)
        out[:, n:] = out[:, n:] * (1.0 - w) + b[:, last - n_seam : last] * w
    return out, engine, last


def _render_overlap(a, b, sr, plan, bass_swap, voc_a=None, voc_b=None):
    """blend and filter: both tracks play together, beat-matched.

    voc_a / voc_b: vocal stems (same shape as a / b). When both are given,
    the voices are taken out of the tracks and get their own volume curves
    (vocal swap). "a" and "b" then mean "everything except the voice".

    Returns (audio, transition start sample, transition end sample,
    bass swap sample or None, stretch engine).
    """
    use_vocals = voc_a is not None and voc_b is not None and plan.vocal_swap is not None
    full_a, full_b = a, b
    if use_vocals:
        a, b = a - voc_a, b - voc_b

    part_a, engine_a = _stretch_a(a, sr, plan)
    part_b, engine_b, b_rest = _stretch_b(b, sr, plan)
    engine = engine_a if engine_a != "none" else engine_b

    n = min(int(round(plan.out_anchors[-1] * sr)), part_a.shape[1], part_b.shape[1])
    tail_b = part_b[:, n:]  # the seam: already blended back into untouched B

    sing_a = sing_b = None
    if use_vocals:
        sing_a = _stretch_a(voc_a, sr, plan)[0][:, :n]
        voice_b = _stretch_b(voc_b, sr, plan)[0]
        sing_b, tail_b = voice_b[:, :n], tail_b + voice_b[:, n : n + tail_b.shape[1]]
        fade_out, fade_in = equal_power_curves(n)
        gate_a, gate_b = vocal_gates(n, sr, int(round(plan.vocal_swap * sr)))
        sing_a = sing_a * fade_out * gate_a
        sing_b = sing_b * fade_in * gate_b

    swap_sample = int(round(plan.bass_swap * sr)) if bass_swap else None
    gains = band_gains(n, sr, swap_sample)
    bands_b = split_bands(part_b[:, :n], sr)
    mixed = np.zeros((2, n))

    if plan.style == "filter":
        # A: the filter removes more and more from the bottom up. Its volume
        # stays up until the last quarter, then fades.
        body_a = part_a[:, :n] if sing_a is None else part_a[:, :n] + sing_a / np.maximum(fade_out, 1e-3)
        thin_a = sweep_highpass(body_a, sr)
        gain_a = np.ones(n)
        fade = int(n * FILTER_FADE_PART)
        if fade > 0:
            gain_a[n - fade :] = equal_power_curves(fade)[0]
        mixed += thin_a * gain_a
        for band in BANDS:
            mixed += bands_b[band] * gains[band][1]
    else:
        bands_a = split_bands(part_a[:, :n], sr)
        for band in BANDS:
            gain_a, gain_b = gains[band]
            mixed += bands_a[band] * gain_a + bands_b[band] * gain_b
        if sing_a is not None:
            mixed += sing_a
    if sing_b is not None:
        mixed += sing_b

    start = int(round(plan.a_start * sr))
    out = np.concatenate(
        [full_a[:, :start], mixed.astype(np.float32), tail_b, full_b[:, b_rest:]], axis=1
    )
    swap = start + swap_sample if swap_sample is not None else None
    return out, start, start + n, swap, engine, start + n + tail_b.shape[1], b_rest


def _render_jump(a, b, sr, plan):
    """cut and echo: A stops on a bar line and B starts on its "one"."""
    cut = min(int(round(plan.a_start * sr)), a.shape[1])
    head = a[:, :cut].copy()
    b_first = int(round(plan.b_start * sr))
    tail = b[:, b_first:].copy()

    fade = min(int(CUT_FADE * sr), head.shape[1], tail.shape[1])
    if fade > 0:
        ramp = np.linspace(0.0, 1.0, fade, dtype=np.float32)
        head[:, -fade:] *= ramp[::-1]
        tail[:, :fade] *= ramp
    end = cut

    if plan.style == "echo" and plan.echo_beat > 0:
        beat = int(round(plan.echo_beat * sr))
        source = a[:, max(0, cut - beat) : cut].astype(np.float64)
        if source.shape[1] > 4 * fade > 0:
            low_cut = butter(2, ECHO_LOW_CUT_HZ, btype="highpass", fs=sr, output="sos")
            source = sosfiltfilt(low_cut, source, axis=-1)
            source[:, :fade] *= np.linspace(0.0, 1.0, fade)
            source[:, -fade:] *= np.linspace(1.0, 0.0, fade)
            for k in range(ECHO_REPEATS):
                at = k * beat
                room = tail.shape[1] - at
                if room <= 0:
                    break
                piece = source[:, :room] * ECHO_FEEDBACK ** (k + 1)
                tail[:, at : at + piece.shape[1]] += piece.astype(np.float32)
            end = cut + min(ECHO_REPEATS * beat, tail.shape[1])

    return np.concatenate([head, tail], axis=1), cut, end, None, "none", end, b_first + (end - cut)


def render_transition(
    audio_a: np.ndarray,
    sr_a: int,
    audio_b: np.ndarray,
    sr_b: int,
    plan: TransitionPlan,
    lufs_a: float | None = None,
    lufs_b: float | None = None,
    bass_swap: bool = True,
    vocals_a: np.ndarray | None = None,
    vocals_b: np.ndarray | None = None,
    normalize: bool = True,
) -> RenderResult:
    """Mix A into B as described by `plan`. Audio arrays are (channels, samples).

    vocals_a / vocals_b: optional vocal stems of the two tracks, same shape
    and sample rate as the tracks. Needed for the vocal swap.

    normalize: limit the result when it would clip. Playlist mode
    switches this off and uses one limiter for the whole set.
    """
    sr = sr_a
    a = _to_stereo(audio_a)
    b = _to_stereo(audio_b)
    overlap = plan.style in ("blend", "filter")
    use_vocals = (
        overlap and vocals_a is not None and vocals_b is not None and plan.vocal_swap is not None
    )
    voc_a = _to_stereo(vocals_a)[:, : a.shape[1]] if use_vocals else None
    voc_b = _to_stereo(vocals_b)[:, : b.shape[1]] if use_vocals else None
    if sr_b != sr:
        b = librosa.resample(b, orig_sr=sr_b, target_sr=sr)
        if use_vocals:
            voc_b = librosa.resample(voc_b, orig_sr=sr_b, target_sr=sr)

    # Loudness matching: bring B to the loudness of A.
    gain_b_db = 0.0
    if lufs_a is not None and lufs_b is not None:
        gain_b_db = float(np.clip(lufs_a - lufs_b, -12.0, 12.0))
        b = b * np.float32(10.0 ** (gain_b_db / 20.0))
        if use_vocals:
            voc_b = voc_b * np.float32(10.0 ** (gain_b_db / 20.0))

    # Key matching: shift B's pitch (all of B, so its key does not jump later).
    b, pitch_engine = pitch_shift(b, sr, plan.pitch_shift_b)
    if use_vocals:
        voc_b = pitch_shift(voc_b, sr, plan.pitch_shift_b)[0]
        voc_a = _fit(voc_a, a.shape[1])
        voc_b = _fit(voc_b, b.shape[1])

    if plan.style in ("cut", "echo"):
        out, start, end, swap, engine, out_resume, b_resume = _render_jump(a, b, sr, plan)
    else:
        out, start, end, swap, engine, out_resume, b_resume = _render_overlap(
            a, b, sr, plan, bass_swap, voc_a, voc_b
        )
    engine = engine if engine != "none" else pitch_engine

    # Two songs added together can go over the digital maximum. A limiter turns
    # the mix down only around the peaks, so the rest keeps its loudness.
    # master_gain_db is the deepest reduction it made (0 when nothing clipped).
    peak = float(np.max(np.abs(out))) if out.size else 0.0
    master_gain_db = 0.0
    if normalize and peak > LIMIT_CEILING:
        out = limit(out, sr, LIMIT_CEILING)
        master_gain_db = float(20.0 * np.log10(LIMIT_CEILING / peak))

    return RenderResult(
        audio=out.astype(np.float32),
        sr=sr,
        transition_start=start / sr,
        transition_end=end / sr,
        bass_swap=swap / sr if swap is not None else None,
        stretch_engine=engine,
        gain_b_db=gain_b_db,
        master_gain_db=master_gain_db,
        style=plan.style,
        vocal_swap=start / sr + plan.vocal_swap if use_vocals else None,
        out_resume=out_resume,
        b_resume=b_resume,
    )
