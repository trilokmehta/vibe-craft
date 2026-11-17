"""3-band EQ for transitions, like the LOW / MID / HIGH knobs on a DJ mixer.

Why: two basslines or two kick drums at the same time sound muddy and loud.
A DJ keeps the bass of only ONE track at any moment and swaps it quickly on a
bar line. Mids and highs can overlap without problems, so they fade slowly.

The audio is split into three bands:
    low   below 200 Hz        (kick drum, bass)
    mid   200 Hz - 2.5 kHz    (vocals, chords)
    high  above 2.5 kHz       (hi-hats, "air")

The filters run forwards and backwards over the audio (zero-phase), so
low + mid + high gives back exactly the original sound. This only works
offline. A real-time player (later) needs Linkwitz-Riley crossover filters.
"""

from __future__ import annotations

import librosa
import numpy as np
from scipy.signal import butter, sosfiltfilt

LOW_HZ = 200.0
HIGH_HZ = 2500.0
BANDS = ("low", "mid", "high")
# The bass swap is a very short fade, not a hard cut, to avoid a click.
BASS_SWAP_FADE = 0.03  # seconds


def split_bands(audio: np.ndarray, sr: int) -> dict[str, np.ndarray]:
    """Split (channels, samples) audio into low, mid and high bands."""
    low_filter = butter(4, LOW_HZ, btype="lowpass", fs=sr, output="sos")
    high_filter = butter(4, HIGH_HZ, btype="highpass", fs=sr, output="sos")
    audio = audio.astype(np.float64)
    low = sosfiltfilt(low_filter, audio, axis=-1)
    high = sosfiltfilt(high_filter, audio, axis=-1)
    mid = audio - low - high
    return {"low": low, "mid": mid, "high": high}


def sweep_highpass(
    audio: np.ndarray, sr: int, start_hz: float = 30.0, end_hz: float = 6000.0, order: int = 2
) -> np.ndarray:
    """High-pass filter whose cut-off frequency rises from start_hz to end_hz
    over the length of `audio` (channels, samples). This is the DJ "filter
    sweep": first the bass disappears, then the body, and only the "air" of
    the track is left.

    The cut-off rises in equal musical steps (octaves per second), not equal
    Hz. It is done on the spectrogram: every short frame is multiplied by the
    filter curve that belongs to its moment in time.
    """
    n = audio.shape[-1]
    spec = librosa.stft(audio.astype(np.float32), n_fft=2048, hop_length=512)
    freqs = np.maximum(librosa.fft_frequencies(sr=sr, n_fft=2048), 1e-3)[:, np.newaxis]
    cutoff = start_hz * (end_hz / start_hz) ** np.linspace(0.0, 1.0, spec.shape[-1])
    # Butterworth high-pass curve: 1 far above the cut-off, falling below it.
    gain = 1.0 / np.sqrt(1.0 + (cutoff[np.newaxis, :] / freqs) ** (2 * order))
    return librosa.istft(spec * gain, hop_length=512, n_fft=2048, length=n)


def equal_power_curves(n: int) -> tuple[np.ndarray, np.ndarray]:
    """Fade-out and fade-in curves of length n.

    "Equal power" means out^2 + in^2 = 1 at every moment, so the total
    loudness stays the same in the middle of the fade. A straight-line fade
    would sound quieter in the middle.
    """
    x = np.linspace(0.0, 1.0, max(n, 1))
    return np.cos(x * np.pi / 2), np.sin(x * np.pi / 2)


VOCAL_SWAP_FADE = 0.25  # seconds; a voice fades out / in this fast at the vocal swap


def vocal_gates(n: int, sr: int, swap_sample: int) -> tuple[np.ndarray, np.ndarray]:
    """On/off curves for the two voices in a transition of n samples.

    A's voice is on until the swap and fades out just before it. B's voice
    is off until the swap and fades in just after it. They never overlap.
    """
    swap = int(np.clip(swap_sample, 0, n))
    fade = int(VOCAL_SWAP_FADE * sr)
    gate_a, gate_b = np.ones(n), np.zeros(n)
    gate_a[swap:], gate_b[swap:] = 0.0, 1.0
    out_from = max(0, swap - fade)
    if swap > out_from:
        gate_a[out_from:swap] = np.linspace(1.0, 0.0, swap - out_from)
    in_to = min(n, swap + fade)
    if in_to > swap:
        gate_b[swap:in_to] = np.linspace(0.0, 1.0, in_to - swap)
    return gate_a, gate_b


def band_gains(
    n: int, sr: int, bass_swap_sample: int | None
) -> dict[str, tuple[np.ndarray, np.ndarray]]:
    """Volume curves for a transition of n samples.

    Returns {band: (gain for track A, gain for track B)}.
    Mid and high: slow equal-power crossfade over the whole transition.
    Low: A keeps the bass until `bass_swap_sample`, then B has it.
    With bass_swap_sample=None the low band fades slowly like the others.
    """
    fade_out, fade_in = equal_power_curves(n)
    gains = {band: (fade_out, fade_in) for band in BANDS}

    if bass_swap_sample is not None:
        end = int(np.clip(bass_swap_sample, 0, n))
        start = max(0, end - int(BASS_SWAP_FADE * sr))
        low_a, low_b = np.ones(n), np.zeros(n)
        low_a[end:], low_b[end:] = 0.0, 1.0
        # The short fade ends exactly on the bar line, so B's first kick is full.
        if end > start:
            low_a[start:end], low_b[start:end] = equal_power_curves(end - start)
        gains["low"] = (low_a, low_b)
    return gains
