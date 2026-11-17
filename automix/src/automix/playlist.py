"""Playlist mode: many songs, one continuous WAV file (Milestone 9).

How the set is built, one pair at a time:

    song 1 ---transition 1---> song 2 ---transition 2---> song 3 ...

* Every song is played from where the transition before it ended to where
  the transition after it starts. Nothing is played twice.
* Every song is made equally loud first (the median loudness of the set).
* If song 2 is pitch-shifted to fit song 1, it stays shifted, so the key of
  transition 2 starts from the key song 2 was really played in.
* A transition may start only a few bars after the previous one ended. If a
  song has no room left for an overlap, a cut is used.
* One limiter works on the whole set, so there is no jump in loudness
  between transitions. The audio is written to the file piece by piece.

A text file with the track list and the time of each transition is written
next to the WAV file (set.wav -> set.tracklist.txt).
"""

from __future__ import annotations

import bisect
import dataclasses
import statistics
from pathlib import Path
from typing import Callable

import librosa
import numpy as np
import soundfile as sf

from .analyzer import DEFAULT_CACHE_DIR, analyze_track
from .audio_io import load_audio
from .planner.order import order_tracks
from .planner.style import plan_transition
from .renderer.crossfade import render_transition
from .renderer.limiter import StreamLimiter
from .renderer.stretch import pitch_shift

SET_SR = 44100  # every song is converted to this sample rate
SOLO_BARS = 8  # a song plays at least this many bars alone between two transitions
MAX_GAIN_DB = 12.0
CEILING = 0.98  # limiter ceiling (about -0.2 dBFS)
MIN_BARS = 8  # songs with fewer bars cannot be mixed


def _stereo(audio: np.ndarray, sr_from: int, sr_to: int) -> np.ndarray:
    if audio.shape[0] == 1:
        audio = np.repeat(audio, 2, axis=0)
    audio = audio[:2]
    if sr_from != sr_to:
        audio = librosa.resample(audio, orig_sr=sr_from, target_sr=sr_to)
    return np.ascontiguousarray(audio, dtype=np.float32)


def _load(path: Path, sr: int) -> np.ndarray:
    audio, sr_file = load_audio(path)
    return _stereo(audio, sr_file, sr)


def _prepare(audio: np.ndarray, sr: int, gain_db: float, semitones: int) -> np.ndarray:
    """Loudness match and key shift for a whole song."""
    out = audio * np.float32(10.0 ** (gain_db / 20.0))
    if semitones:
        out = pitch_shift(out, sr, semitones)[0]
    return out


def _first_bar_at(downbeats: list[float], seconds: float) -> int:
    return bisect.bisect_left(downbeats, seconds)


def _fmt(seconds: float) -> str:
    minutes, sec = divmod(int(round(seconds)), 60)
    hours, minutes = divmod(minutes, 60)
    return f"{hours}:{minutes:02d}:{sec:02d}" if hours else f"{minutes}:{sec:02d}"


def mix_playlist(
    paths: list[str | Path],
    out_path: str | Path,
    bars: int = 16,
    cache_dir: str | Path | None = DEFAULT_CACHE_DIR,
    beat_backend: str = "auto",
    bass_swap: bool = True,
    key_match: bool = True,
    smart: bool = True,
    style: str = "auto",
    vocals: bool = False,
    order: str = "given",
    limit: int | None = None,
    progress: Callable[[str], None] | None = None,
    sr: int = SET_SR,
) -> dict:
    """Mix `paths` into one continuous WAV file.

    order: "given" keeps the order of `paths`; "smart" reorders them so that
    neighbours are easy to mix (see planner/order.py).
    limit: only use the first `limit` songs.
    progress: called with a line of text at every step.

    Returns {"out", "tracklist", "duration", "entries"}. Every entry is a dict
    for one song: its start time in the set and how it was mixed in.
    """
    say = progress or (lambda message: None)
    if order not in ("given", "smart"):
        raise ValueError(f"Unknown order: {order}")
    if vocals and not cache_dir:
        raise ValueError("Vocal-aware mixing needs a cache folder.")

    paths = [Path(p) for p in paths][: limit or None]
    songs: list[tuple[Path, dict]] = []
    for path in paths:
        try:
            analysis = analyze_track(path, cache_dir=cache_dir, beat_backend=beat_backend)
        except Exception as exc:
            say(f"Skipped {path.name}: could not analyze it ({exc})")
            continue
        if len(analysis["downbeats"]) < MIN_BARS + 1:
            say(f"Skipped {path.name}: fewer than {MIN_BARS} bars found")
            continue
        songs.append((path, analysis))
    if len(songs) < 2:
        raise ValueError("A playlist needs at least two songs that can be analyzed.")

    if order == "smart":
        songs = [songs[i] for i in order_tracks([a for _, a in songs])]
    if vocals:
        from .analyzer.stems import load_vocals, vocal_analysis

        for path, analysis in songs:
            analysis["vocals"] = vocal_analysis(path, analysis)

    # Every song gets the same loudness: the median of the set.
    lufs = [a["loudness_lufs"] for _, a in songs if a["loudness_lufs"] is not None]
    target = statistics.median(lufs) if lufs else None

    def gain_for(analysis: dict) -> float:
        if target is None or analysis["loudness_lufs"] is None:
            return 0.0
        return float(np.clip(target - analysis["loudness_lufs"], -MAX_GAIN_DB, MAX_GAIN_DB))

    def load_song(index: int, gain_db: float, semitones: int):
        path, analysis = songs[index]
        audio = _prepare(_load(path, sr), sr, gain_db, semitones)
        voice = None
        if vocals:
            stem, stem_sr = load_vocals(analysis["vocals"])
            voice = _prepare(_stereo(stem, stem_sr, sr), sr, gain_db, semitones)
        return audio, voice

    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    limiter = StreamLimiter(CEILING, sr)
    written = 0  # samples handed to the limiter so far
    entries: list[dict] = []

    first_path, current = songs[0]
    say(f"[1/{len(songs)}] {first_path.name}")
    cur_audio, cur_voice = load_song(0, gain_for(current), 0)
    entries.append(_entry(first_path, current, 0.0, None))
    emit_from = 0  # sample of the current song where its own part of the set begins

    with sf.SoundFile(str(out_path), "w", samplerate=sr, channels=2, subtype="PCM_16") as file:

        def write(chunk: np.ndarray) -> None:
            nonlocal written
            written += chunk.shape[1]
            ready = limiter.push(chunk)
            if ready.shape[1]:
                file.write(np.clip(ready, -1.0, 1.0).T)

        for k in range(1, len(songs)):
            path, nxt = songs[k]
            min_bar = 0
            if k > 1:
                min_bar = _first_bar_at(current["downbeats"], emit_from / sr) + SOLO_BARS
            plan = plan_transition(
                current, nxt, bars=bars, key_match=key_match, style=style, smart=smart,
                min_bar_a=min_bar,
            )
            say(
                f"[{k + 1}/{len(songs)}] {path.name}  <- {plan.style}"
                + (f", {plan.bars_a} bars" if plan.bars_a else "")
                + (f", pitch {plan.pitch_shift_b:+d}" if plan.pitch_shift_b else "")
                + f"  ({plan.style_reason})"
            )

            next_audio, next_voice = load_song(k, gain_for(nxt), plan.pitch_shift_b)
            result = render_transition(
                cur_audio, sr, next_audio, sr,
                dataclasses.replace(plan, pitch_shift_b=0),
                bass_swap=bass_swap, vocals_a=cur_voice, vocals_b=next_voice, normalize=False,
            )

            start = int(round(result.transition_start * sr))
            first = emit_from
            if start < emit_from:  # the song is too short for the planned start
                say("  warning: this song is too short for a clean transition here")
                first = start
            entry_time = (written + start - first) / sr
            warnings = [
                f"bar lines of the {name} track are uneven in the transition"
                for name, steady in (("first", plan.steady_a), ("second", plan.steady_b))
                if not steady
            ]
            entries.append(_entry(path, nxt, entry_time, plan, warnings))

            write(result.audio[:, first : result.out_resume])
            cur_audio, cur_voice = next_audio, next_voice
            emit_from = result.b_resume
            current = nxt
            if plan.pitch_shift_b and nxt.get("key"):
                current = {**nxt, "key": {**nxt["key"], "camelot": plan.key_b_played}}
            del result

        write(cur_audio[:, emit_from:])
        tail = limiter.flush()
        if tail.shape[1]:
            file.write(np.clip(tail, -1.0, 1.0).T)

    duration = written / sr
    tracklist = out_path.with_suffix(".tracklist.txt")
    tracklist.write_text(_tracklist_text(out_path, entries, duration), encoding="utf-8")
    return {
        "out": str(out_path),
        "tracklist": str(tracklist),
        "duration": duration,
        "entries": entries,
    }


def _entry(path: Path, analysis: dict, time: float, plan, warnings: list[str] | None = None) -> dict:
    entry = {
        "file": str(path),
        "title": path.stem,
        "time": time,
        "bpm": analysis["bpm"],
        "key": analysis["key"]["camelot"] if analysis.get("key") else "",
        "style": None,
        "warnings": warnings or [],
    }
    if plan is not None:
        entry.update(
            style=plan.style,
            reason=plan.style_reason,
            bars=plan.bars_a,
            pitch_shift=plan.pitch_shift_b,
            key_played=plan.key_b_played,
            key_score=plan.key_score,
            score=plan.score,
            vocal_clash=plan.vocal_clash,
        )
    return entry


def _tracklist_text(out_path: Path, entries: list[dict], duration: float) -> str:
    lines = [f"{out_path.name}: {len(entries)} songs, {_fmt(duration)}", ""]
    for number, e in enumerate(entries, 1):
        key = e["key"]
        if e.get("pitch_shift"):
            key = f"{e['key']} -> {e['key_played']}"
        lines.append(f"{_fmt(e['time']):>7}  {number:2d}. {e['title']}   ({e['bpm']:.0f} BPM, {key})")
        if e["style"]:
            detail = e["style"] + (f", {e['bars']} bars" if e.get("bars") else "")
            lines.append(f"{'':>7}      mixed in with {detail}: {e['reason']}")
        for warning in e["warnings"]:
            lines.append(f"{'':>7}      warning: {warning}")
    return "\n".join(lines) + "\n"
