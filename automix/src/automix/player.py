"""Real-time player: plays a playlist while it is still being mixed.

    automix play my_folder/ --order smart

A background thread runs the playlist mixer (the same one that writes a WAV
file). Its finished audio goes into a buffer that stays about `buffer_seconds`
ahead of the speakers. The sound card pulls audio out of the buffer in real
time. When the buffer is full the mixer waits, so the next transition is
always rendered shortly before it is needed.

This is not live DSP: every transition is rendered with the full-quality
offline code, a little ahead of time. That is why it sounds exactly like the
WAV file would.

What it cannot do: skip ahead or jump back (the set is planned in order), and
the first start needs every song analyzed (instant when they are cached).
"""

from __future__ import annotations

import collections
import threading
import time
from pathlib import Path
from typing import Callable

import numpy as np

from .playlist import SET_SR, mix_playlist

BLOCK_SECONDS = 0.5  # the mixer's audio is cut into pieces of this length
DEFAULT_BUFFER_SECONDS = 60.0  # how far ahead of the speakers the mixer works
PREBUFFER_SECONDS = 4.0  # playback starts when this much audio is ready


class Stopped(Exception):
    """Raised inside the mixer thread to end it when playback is stopped."""


class AudioBuffer:
    """A queue of audio between the mixer thread and the sound card.

    `put` blocks while the buffer is full (that is what paces the mixer).
    `read` never blocks: it pads with silence when the buffer runs dry, so the
    sound card callback is always on time.
    """

    def __init__(self, sr: int = SET_SR, seconds: float = DEFAULT_BUFFER_SECONDS):
        self.sr = sr
        self.capacity = int(seconds * sr)
        self._blocks: collections.deque[np.ndarray] = collections.deque()
        self._queued = 0  # samples in the deque
        self._offset = 0  # samples of the first block that were already read
        self._cond = threading.Condition()
        self._closed = False  # the producer has nothing more to add
        self._stopped = False
        self.played = 0  # samples handed to the sound card
        self.underruns = 0  # callbacks that had to be padded with silence

    # ---- producer side
    def put(self, audio: np.ndarray) -> None:
        """Add (2, samples) audio. Waits while the buffer is full."""
        step = int(BLOCK_SECONDS * self.sr)
        for start in range(0, audio.shape[1], step):
            block = np.ascontiguousarray(audio[:, start : start + step].T, dtype=np.float32)
            with self._cond:
                while self._queued >= self.capacity and not self._stopped:
                    self._cond.wait(0.2)
                if self._stopped:
                    raise Stopped()
                self._blocks.append(block)
                self._queued += block.shape[0]
                self._cond.notify_all()

    def close(self) -> None:
        with self._cond:
            self._closed = True
            self._cond.notify_all()

    def stop(self) -> None:
        with self._cond:
            self._stopped = True
            self._cond.notify_all()

    # ---- consumer side
    @property
    def queued_seconds(self) -> float:
        return (self._queued - self._offset) / self.sr

    @property
    def finished(self) -> bool:
        """The set is over: everything was produced and played."""
        with self._cond:
            return self._closed and not self._blocks

    def wait_ready(self, seconds: float, timeout: float | None = None) -> bool:
        """Wait until `seconds` of audio is buffered (or the set is shorter)."""
        need = int(seconds * self.sr)
        end = None if timeout is None else time.monotonic() + timeout
        with self._cond:
            while (self._queued - self._offset) < need and not self._closed and not self._stopped:
                left = None if end is None else end - time.monotonic()
                if left is not None and left <= 0:
                    return False
                self._cond.wait(0.2 if left is None else min(0.2, left))
            return not self._stopped

    def read(self, frames: int) -> np.ndarray:
        """(frames, 2) float32 audio. Silence is added when the buffer is empty."""
        out = np.zeros((frames, 2), dtype=np.float32)
        filled = 0
        with self._cond:
            while filled < frames and self._blocks:
                block = self._blocks[0]
                take = min(frames - filled, block.shape[0] - self._offset)
                out[filled : filled + take] = block[self._offset : self._offset + take]
                filled += take
                self._offset += take
                if self._offset >= block.shape[0]:
                    self._queued -= block.shape[0]
                    self._blocks.popleft()
                    self._offset = 0
            self._queued = max(self._queued, 0)
            if filled < frames and not self._closed:
                self.underruns += 1
            self.played += filled
            self._cond.notify_all()
        return out


class Player:
    """Mixes `paths` in a background thread and plays them as they are ready."""

    def __init__(
        self,
        paths: list[str | Path],
        buffer_seconds: float = DEFAULT_BUFFER_SECONDS,
        sr: int = SET_SR,
        progress: Callable[[str], None] | None = None,
        **mix_options,
    ):
        self.paths = paths
        self.sr = sr
        self.buffer = AudioBuffer(sr, buffer_seconds)
        self.entries: list[dict] = []
        self.error: Exception | None = None
        self.report: dict | None = None
        self._progress = progress or (lambda message: None)
        self._options = mix_options
        self._thread = threading.Thread(target=self._run, name="automix-mixer", daemon=True)

    def start(self) -> None:
        self._thread.start()

    def _run(self) -> None:
        try:
            self.report = mix_playlist(
                self.paths,
                None,
                sr=self.sr,
                sink=self.buffer.put,
                on_entry=self.entries.append,
                progress=self._progress,
                **self._options,
            )
        except Stopped:
            pass
        except Exception as exc:  # shown to the person by the caller
            self.error = exc
        finally:
            self.buffer.close()

    def stop(self) -> None:
        self.buffer.stop()

    def now_playing(self) -> dict | None:
        """The entry of the song that is audible now."""
        seconds = self.buffer.played / self.sr
        current = None
        for entry in list(self.entries):
            if entry["time"] <= seconds:
                current = entry
        return current

    def play(
        self,
        stream_factory: Callable | None = None,
        on_song: Callable[[dict], None] | None = None,
        poll: float = 0.25,
    ) -> None:
        """Play until the set ends or `stop()` / Ctrl+C. Blocks.

        stream_factory(samplerate, callback) must return an object with
        start(), stop() and close(). The default uses sounddevice. Tests pass a
        fake one.
        """
        factory = stream_factory or _sounddevice_stream
        self.start()
        self.buffer.wait_ready(PREBUFFER_SECONDS)
        if self.error:
            raise self.error
        if self.buffer.finished:  # nothing was produced
            if self.error:
                raise self.error
            return

        def callback(outdata, frames, time_info, status):  # runs on the audio thread
            outdata[:] = self.buffer.read(frames)

        stream = factory(self.sr, callback)
        shown = None
        stream.start()
        try:
            while not self.buffer.finished and not self.buffer._stopped:
                song = self.now_playing()
                if song is not shown and song is not None:
                    shown = song
                    if on_song:
                        on_song(song)
                time.sleep(poll)
            time.sleep(0.4)  # let the sound card play out what it already holds
        except KeyboardInterrupt:
            pass
        finally:
            self.stop()
            stream.stop()
            stream.close()
        if self.error:
            raise self.error


def _sounddevice_stream(sr: int, callback, device=None):
    try:
        import sounddevice as sd
    except ImportError as exc:
        raise RuntimeError(
            "The player needs sounddevice. Install it with: pip install sounddevice"
        ) from exc
    return sd.OutputStream(
        samplerate=sr, channels=2, dtype="float32", callback=callback, device=device, blocksize=1024
    )


def list_devices() -> str:
    import sounddevice as sd

    return str(sd.query_devices())
