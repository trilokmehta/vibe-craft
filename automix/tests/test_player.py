"""Real-time player: the buffer, and a whole set played through a fake sound card."""

import threading

import numpy as np
import soundfile as sf
from synth import SR, make_track

from automix.player import AudioBuffer, Player, Stopped
from automix.playlist import mix_playlist


def test_buffer_hands_out_audio_in_order_and_pads_with_silence():
    buf = AudioBuffer(sr=100, seconds=10)
    audio = np.arange(2 * 130, dtype=np.float32).reshape(2, 130) / 1000
    buf.put(audio)
    first = buf.read(100)
    assert np.array_equal(first, audio[:, :100].T)
    second = buf.read(100)  # only 30 samples left
    assert np.array_equal(second[:30], audio[:, 100:].T)
    assert np.all(second[30:] == 0) and buf.underruns == 1
    assert buf.played == 130


def test_full_buffer_makes_the_producer_wait_and_stop_frees_it():
    buf = AudioBuffer(sr=100, seconds=1)  # holds 100 samples
    result = []

    def produce():
        try:
            buf.put(np.zeros((2, 1000), dtype=np.float32))
            result.append("done")
        except Stopped:
            result.append("stopped")

    thread = threading.Thread(target=produce)
    thread.start()
    thread.join(0.5)
    assert thread.is_alive()  # blocked: the buffer is full and nobody reads
    buf.stop()
    thread.join(2)
    assert result == ["stopped"]


def test_finished_only_after_close_and_empty():
    buf = AudioBuffer(sr=100, seconds=5)
    buf.put(np.zeros((2, 50), dtype=np.float32))
    buf.close()
    assert not buf.finished
    buf.read(50)
    assert buf.finished


class FakeCard:
    """Pulls audio like a sound card would, but as fast as the buffer allows."""

    def __init__(self, buffer_ref):
        self.buffer_ref = buffer_ref
        self.audio = []
        self._thread = None
        self._run = False

    def __call__(self, sr, callback):
        self.callback = callback
        return self

    def start(self):
        self._run = True
        self._thread = threading.Thread(target=self._pull, daemon=True)
        self._thread.start()

    def _pull(self):
        buf = self.buffer_ref()
        while self._run and not buf.finished:
            if not buf.wait_ready(0.05, timeout=0.2) and not buf._closed:
                continue
            out = np.zeros((1024, 2), dtype=np.float32)
            self.callback(out, 1024, None, None)
            self.audio.append(out)

    def stop(self):
        self._run = False
        if self._thread:
            self._thread.join(2)

    def close(self):
        pass


def test_player_plays_exactly_what_the_wav_mix_contains(tmp_path):
    for name, bpm, shift in (("a", 120.0, 0), ("b", 122.0, 0), ("c", 124.0, 2)):
        sf.write(tmp_path / f"{name}.wav", make_track(bpm, 56, semitones=shift), SR)
    paths = [tmp_path / f"{n}.wav" for n in "abc"]
    options = dict(cache_dir=tmp_path / "cache", beat_backend="librosa")

    expected = mix_playlist(paths, tmp_path / "set.wav", sr=SR, **options)
    wav, _ = sf.read(tmp_path / "set.wav", dtype="float32")

    player = Player(paths, buffer_seconds=5, sr=SR, **options)
    card = FakeCard(lambda: player.buffer)
    titles = []
    player.play(stream_factory=card, on_song=lambda e: titles.append(e["title"]), poll=0.01)

    played = np.concatenate(card.audio)
    n = wav.shape[0]
    # 16-bit rounding in the file; everything after the end is silence padding
    assert np.allclose(played[:n], wav, atol=2e-4)
    assert np.all(played[n:] == 0)
    assert [e["title"] for e in player.entries] == ["a", "b", "c"]
    assert titles and titles[0] == "a"
    assert abs(player.buffer.played / SR - expected["duration"]) < 1.0
