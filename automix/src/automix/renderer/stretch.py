"""Time-stretching: change the speed of audio without changing its pitch.

The speed can change over time. It is described by a "time map": a list of
anchor points that say "this moment of the input must land at that moment of
the output". Between two anchors the speed is constant.

Best quality: the Rubber Band command-line program (through pyrubberband).
If `rubberband` is not on PATH, we fall back to our own phase vocoder. That
works everywhere but sounds a little "phasey" (smeared drums), so install
Rubber Band for real use.
"""

from __future__ import annotations

import shutil

import librosa
import numpy as np

N_FFT = 2048
HOP = 512


def has_rubberband() -> bool:
    return shutil.which("rubberband") is not None


def stretch_to_map(
    audio: np.ndarray, sr: int, src_times: np.ndarray, dst_times: np.ndarray
) -> tuple[np.ndarray, str]:
    """Stretch `audio` (channels, samples) so that the input moment
    src_times[k] (seconds) is heard at the output moment dst_times[k].

    Both lists must start at 0 and go up. src_times[-1] must be the input
    length. The output is exactly dst_times[-1] long.
    Returns (stretched audio, name of the engine used).
    """
    src = np.round(np.asarray(src_times, dtype=float) * sr).astype(np.int64)
    dst = np.round(np.asarray(dst_times, dtype=float) * sr).astype(np.int64)
    src[-1] = audio.shape[1]
    n_out = int(dst[-1])

    if np.max(np.abs(src - dst)) <= 1:
        return _fit_length(audio, n_out), "none"

    # Remove anchors that do not move forward (the engines need rising values).
    keep = np.ones(len(src), dtype=bool)
    keep[1:] = (np.diff(src) > 0) & (np.diff(dst) > 0)
    keep[-1] = True
    src, dst = src[keep], dst[keep]

    if has_rubberband():
        import pyrubberband

        time_map = [(int(a), int(b)) for a, b in zip(src, dst)]
        out = pyrubberband.timemap_stretch(audio.T, sr, time_map)
        out = out[np.newaxis, :] if out.ndim == 1 else out.T
        return _fit_length(out.astype(np.float32), n_out), "rubberband"

    out = _phase_vocoder_map(audio, src, dst)
    return out, "built-in phase vocoder (low quality, install Rubber Band)"


def time_stretch(audio: np.ndarray, sr: int, rate: float) -> tuple[np.ndarray, str]:
    """Play `audio` (channels, samples) `rate` times faster, at one fixed speed."""
    length = audio.shape[1] / sr
    return stretch_to_map(audio, sr, np.array([0.0, length]), np.array([0.0, length / rate]))


def pitch_shift(audio: np.ndarray, sr: int, semitones: float) -> tuple[np.ndarray, str]:
    """Move the pitch of `audio` (channels, samples) up or down. Speed and
    length stay the same. Used to bring a track into a key that fits.

    How: stretch the audio in time, then play it back faster or slower by
    resampling. The two changes cancel in speed, and only the pitch moves.
    """
    if abs(semitones) < 1e-6:
        return audio, "none"
    n = audio.shape[1]
    if has_rubberband():
        import pyrubberband

        out = pyrubberband.pitch_shift(audio.T, sr, semitones)
        out = out[np.newaxis, :] if out.ndim == 1 else out.T
        return _fit_length(out.astype(np.float32), n), "rubberband"
    out = librosa.effects.pitch_shift(audio, sr=sr, n_steps=semitones)
    return _fit_length(out.astype(np.float32), n), "librosa (low quality, install Rubber Band)"


def _fit_length(audio: np.ndarray, n: int) -> np.ndarray:
    if audio.shape[1] >= n:
        return audio[:, :n]
    return np.pad(audio, ((0, 0), (0, n - audio.shape[1])))


def _phase_vocoder_map(audio: np.ndarray, src: np.ndarray, dst: np.ndarray) -> np.ndarray:
    """Phase vocoder with a changing speed (after Flanagan & Golden, 1966;
    same idea as librosa.phase_vocoder, which only supports a fixed speed).

    The sound is cut into short overlapping frames. To change speed we read
    the frames faster or slower than they were made, and repair the phase of
    every frequency so the frames still join smoothly.
    """
    n_out = int(dst[-1])
    spec = librosa.stft(audio, n_fft=N_FFT, hop_length=HOP)  # (channels, bins, frames)
    n_frames = spec.shape[-1]

    # For every output frame: which (fractional) input frame should be heard?
    out_pos = np.arange(0, n_out + HOP, HOP)
    steps = np.interp(out_pos, dst, src) / HOP
    steps = np.clip(steps, 0, n_frames - 1)

    spec = np.pad(spec, ((0, 0), (0, 0), (0, 2)))
    magnitude, phase = np.abs(spec), np.angle(spec)
    # Phase advance per hop expected for the centre frequency of each bin.
    expected = np.linspace(0, np.pi * HOP, spec.shape[1])[np.newaxis, :]

    out = np.empty((spec.shape[0], spec.shape[1], len(steps)), dtype=np.complex64)
    phase_acc = phase[:, :, int(steps[0])].copy()
    for t, step in enumerate(steps):
        i = int(step)
        frac = step - i
        mag = (1.0 - frac) * magnitude[:, :, i] + frac * magnitude[:, :, i + 1]
        out[:, :, t] = mag * np.exp(1j * phase_acc)
        # How far the phase really moved between the two input frames.
        dphase = phase[:, :, i + 1] - phase[:, :, i] - expected
        dphase -= 2.0 * np.pi * np.round(dphase / (2.0 * np.pi))
        phase_acc += expected + dphase

    y = librosa.istft(out, hop_length=HOP, n_fft=N_FFT, length=n_out)
    return y.astype(np.float32)
