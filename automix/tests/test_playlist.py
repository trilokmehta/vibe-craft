"""Playlist mode: limiter, track order, planning limits and a full set."""

import numpy as np
import pytest
import soundfile as sf
from synth import SR, make_track
from test_smart import LOUD, QUIET, fake

from automix.planner.order import order_tracks, pair_score
from automix.planner.style import plan_transition
from automix.playlist import mix_playlist
from automix.renderer.limiter import StreamLimiter, limit


# ----------------------------------------------------------------- limiter
def test_limiter_keeps_peaks_under_the_ceiling():
    rng = np.random.default_rng(1)
    audio = (rng.standard_normal((2, 5 * SR)) * 0.5).astype(np.float32)
    audio[:, 2 * SR : 2 * SR + 50] *= 4.0  # a loud moment
    out = limit(audio, SR, ceiling=0.9)
    assert out.shape == audio.shape
    assert np.max(np.abs(out)) <= 0.9 + 1e-5


def test_limiter_leaves_quiet_audio_alone():
    audio = (0.3 * np.sin(2 * np.pi * 440 * np.arange(3 * SR) / SR)).astype(np.float32)[np.newaxis]
    audio = np.repeat(audio, 2, axis=0)
    assert np.allclose(limit(audio, SR), audio, atol=1e-6)


def test_limiter_only_turns_down_around_the_peak():
    audio = np.full((2, 4 * SR), 0.2, dtype=np.float32)
    audio[:, 2 * SR] = 2.0
    out = limit(audio, SR)
    assert np.max(np.abs(out)) <= 0.98 + 1e-5
    assert np.allclose(out[:, : SR // 2], 0.2) and np.allclose(out[:, -SR // 2 :], 0.2)


def test_limiter_in_pieces_gives_the_same_length_and_no_overshoot():
    rng = np.random.default_rng(2)
    audio = (rng.standard_normal((2, 3 * SR)) * 0.6).astype(np.float32)
    streaming = StreamLimiter(0.95, SR)
    pieces = [streaming.push(audio[:, a : a + 7001]) for a in range(0, audio.shape[1], 7001)]
    pieces.append(streaming.flush())
    out = np.concatenate(pieces, axis=1)
    assert out.shape == audio.shape
    assert np.max(np.abs(out)) <= 0.95 + 1e-5
    assert np.allclose(out, limit(audio, SR, 0.95), atol=1e-5)  # the same as in one piece


# ------------------------------------------------------------------- order
def test_pair_score_and_order():
    def track(bpm, key):
        return {"bpm": bpm, "key": {"camelot": key, "confidence": 0.5}}

    tracks = [track(120, "8A"), track(140, "2A"), track(122, "8B"), track(121, "9A")]
    assert pair_score(tracks[0], tracks[3]) > pair_score(tracks[0], tracks[1])
    assert pair_score(track(70, "8A"), track(140, "8A")) == 1.0  # double time counts
    order = order_tracks(tracks)
    assert order[0] == 0 and sorted(order) == [0, 1, 2, 3]
    assert order[-1] == 1  # the clashing song goes last


# ---------------------------------------------------------------- planning
def test_min_bar_moves_the_transition_later():
    a, b = fake([LOUD] * 8), fake([LOUD] * 6)
    free = plan_transition(a, b, style="blend")
    assert free.bar_a == 56
    late = plan_transition(fake([LOUD] * 8), fake([LOUD] * 6), style="blend", min_bar_a=40)
    assert late.bar_a >= 40


def test_cut_when_there_is_no_room_for_an_overlap():
    plan = plan_transition(fake([LOUD] * 8), fake([LOUD] * 6), min_bar_a=60)
    assert plan.style == "cut" and "not enough music" in plan.style_reason
    assert plan.bar_a >= 60
    forced = plan_transition(fake([LOUD] * 8), fake([LOUD] * 6), style="echo", min_bar_a=60)
    assert forced.style == "echo"  # a style chosen by the user stays


# --------------------------------------------------------------- full set
@pytest.fixture(scope="module")
def songs(tmp_path_factory):
    folder = tmp_path_factory.mktemp("set")
    for name, bpm, shift in (("a", 120.0, 0), ("b", 122.0, 0), ("c", 124.0, 2)):
        sf.write(folder / f"{name}.wav", make_track(bpm, 56, semitones=shift), SR)
    return folder


def test_a_full_set(songs):
    out = songs / "set.wav"
    messages = []
    report = mix_playlist(
        [songs / "a.wav", songs / "b.wav", songs / "c.wav"],
        out,
        cache_dir=songs / "cache",
        beat_backend="librosa",
        progress=messages.append,
        sr=SR,
    )
    audio, sr = sf.read(out)
    assert sr == SR and audio.ndim == 2
    assert np.isfinite(audio).all() and np.max(np.abs(audio)) <= 0.99

    # The report matches the file.
    assert report["duration"] == pytest.approx(len(audio) / sr, abs=0.001)
    entries = report["entries"]
    assert [e["title"] for e in entries] == ["a", "b", "c"]
    times = [e["time"] for e in entries]
    assert times[0] == 0.0 and times == sorted(times) and times[-1] < report["duration"]
    assert entries[0]["style"] is None and all(e["style"] for e in entries[1:])
    assert len(messages) == 3

    # No gap and no restart: every second of the set has sound in it.
    # (The test songs have a quiet intro, about 0.01 RMS. A gap would be 0.)
    level = [
        float(np.sqrt(np.mean(audio[a * sr : (a + 1) * sr] ** 2)))
        for a in range(int(report["duration"]) - 2)
    ]
    assert min(level) > 0.004
    # The set is shorter than the three songs together (the transitions overlap).
    assert report["duration"] < 3 * 112.0

    text = (songs / "set.tracklist.txt").read_text(encoding="utf-8")
    assert "1. a" in text and "3. c" in text and "mixed in with" in text


def test_smart_order_and_limit(songs):
    report = mix_playlist(
        [songs / "a.wav", songs / "b.wav", songs / "c.wav"],
        songs / "set2.wav",
        cache_dir=songs / "cache",
        beat_backend="librosa",
        order="smart",
        limit=2,
        sr=SR,
    )
    assert [e["title"] for e in report["entries"]] == ["a", "b"]


def test_playlist_needs_two_songs(songs):
    with pytest.raises(ValueError):
        mix_playlist([songs / "a.wav"], songs / "x.wav", cache_dir=songs / "cache", beat_backend="librosa")
