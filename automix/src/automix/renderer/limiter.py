"""Limiter: keeps a whole mix below the digital maximum without turning
everything down.

When two songs play together the sum can go over 0 dBFS for a moment. Turning
the whole mix down for that one moment would make the set quiet. A limiter
turns down only around the peaks, smoothly.

How it works (a "look-ahead" limiter):
1. For every sample, the gain that would bring it exactly to the ceiling.
2. Minimum filter over a short window: the gain is already low when a peak
   is still coming.
3. Moving average over the same window: the gain moves smoothly (no clicks).
Because of steps 2 and 3 the final gain is never higher than the gain needed
for any sample, so nothing goes over the ceiling.

`StreamLimiter` works on pieces, so a set of hours does not have to fit in
memory. It holds back the last window of audio until it knows what follows.
"""

from __future__ import annotations

import numpy as np
from scipy.ndimage import minimum_filter1d, uniform_filter1d


class StreamLimiter:
    def __init__(
        self, ceiling: float = 0.98, sr: int = 44100, window_seconds: float = 0.02, channels: int = 2
    ):
        window = max(3, int(window_seconds * sr))
        self.window = window + (1 - window % 2)  # odd
        self.ceiling = ceiling
        self.channels = channels
        self.context = np.zeros((channels, 0), dtype=np.float32)  # already written
        self.pending = np.zeros((channels, 0), dtype=np.float32)  # waiting for more audio

    def _gain(self, data: np.ndarray) -> np.ndarray:
        peak = np.max(np.abs(data), axis=0)
        needed = np.minimum(1.0, self.ceiling / np.maximum(peak, 1e-9))
        low = minimum_filter1d(needed, size=self.window, mode="nearest")
        return uniform_filter1d(low, size=self.window, mode="nearest").astype(np.float32)

    def push(self, chunk: np.ndarray) -> np.ndarray:
        """Add audio (channels, samples). Returns the audio that is ready."""
        data = np.concatenate([self.context, self.pending, chunk.astype(np.float32)], axis=1)
        first = self.context.shape[1]
        end = data.shape[1] - self.window
        if end <= first:
            self.pending = data[:, first:]
            return np.zeros((self.channels, 0), dtype=np.float32)
        gain = self._gain(data)
        out = data[:, first:end] * gain[first:end]
        self.context = data[:, max(0, end - self.window) : end]
        self.pending = data[:, end:]
        return out

    def flush(self) -> np.ndarray:
        """Return the audio that is still held back. Call once at the end."""
        data = np.concatenate([self.context, self.pending], axis=1)
        first = self.context.shape[1]
        self.context = np.zeros((self.channels, 0), dtype=np.float32)
        self.pending = np.zeros((self.channels, 0), dtype=np.float32)
        if data.shape[1] <= first:
            return np.zeros((self.channels, 0), dtype=np.float32)
        return data[:, first:] * self._gain(data)[first:]


def limit(audio: np.ndarray, sr: int = 44100, ceiling: float = 0.98) -> np.ndarray:
    """Limit a whole (channels, samples) array at once."""
    limiter = StreamLimiter(ceiling, sr, channels=audio.shape[0])
    return np.concatenate([limiter.push(audio), limiter.flush()], axis=1)
