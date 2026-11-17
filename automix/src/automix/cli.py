"""Command line interface.

    automix analyze song.mp3
    automix analyze my_folder/ --beats librosa

    automix a.mp3 b.mp3 -o out.wav
    automix a.mp3 b.mp3 -o check.wav --preview 20

    automix my_folder/ -o set.wav
    automix a.mp3 b.mp3 c.mp3 d.mp3 -o set.wav --order smart
"""

from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path


def _fmt_time(seconds: float) -> str:
    minutes, sec = divmod(seconds, 60.0)
    return f"{int(minutes)}:{sec:05.2f}"


def print_analysis(result: dict) -> None:
    key = result["key"]
    lufs = result["loudness_lufs"]
    print(f"File      : {Path(result['file']).name}")
    print(f"Duration  : {_fmt_time(result['duration'])}")
    print(f"BPM       : {result['bpm']:.2f}   (beats from: {result['beat_backend']})")
    print(f"Key       : {key['name']}  [Camelot {key['camelot']}]  confidence {key['confidence']:.2f}")
    print(f"Loudness  : {'n/a' if lufs is None else f'{lufs:.1f} LUFS'}   peak {result['peak_db']:.1f} dBFS")
    print(f"Beats     : {len(result['beats'])}   Downbeats (bars): {len(result['downbeats'])}")
    first = ", ".join(_fmt_time(t) for t in result["downbeats"][:4])
    print(f"First downbeats: {first}")
    print()
    print("Phrases:")
    print("   #    start      end  bars  energy  section")
    for p in result["phrases"]:
        print(
            f"  {p['index']:2d}  {_fmt_time(p['start']):>7}  {_fmt_time(p['end']):>7}"
            f"  {p['bars']:4d}  {p['energy_db']:6.1f}  {p['label']}"
        )
    print()
    state = "loaded from cache" if result.get("from_cache") else "saved"
    if result.get("cache_file"):
        print(f"JSON {state}: {result['cache_file']}")


def _cmd_analyze(args: argparse.Namespace) -> int:
    # Imported here so that "automix --help" stays fast.
    from .analyzer import analyze_track
    from .audio_io import find_audio_files

    files = find_audio_files(args.inputs)
    if not files:
        print("No audio files found.", file=sys.stderr)
        return 1

    failed = 0
    for i, path in enumerate(files):
        if i:
            print("-" * 60)
        started = time.time()
        try:
            result = analyze_track(
                path,
                cache_dir=args.cache_dir,
                beat_backend=args.beats,
                use_cache=not args.no_cache,
            )
        except Exception as exc:
            print(f"Could not analyze {path.name}: {exc}", file=sys.stderr)
            failed += 1
            continue
        print_analysis(result)
        if args.vocals:
            from .analyzer.stems import vocal_analysis

            activity = vocal_analysis(path, result)["bar_activity"]
            sung = sum(1 for v in activity if v >= 0.5)
            print(f"Vocals    : singing in {sung} of {len(activity)} bars")
        print(f"Time: {time.time() - started:.1f} s")
    return 1 if failed else 0


def _cmd_mix(args: argparse.Namespace) -> int:
    from .mix import mix_two_tracks

    # A folder, or more than two files, is a playlist.
    if len(args.inputs) != 2 or any(Path(p).is_dir() for p in args.inputs):
        return _cmd_playlist(args)
    for path in args.inputs:
        if not Path(path).is_file():
            print(f"File not found: {path}", file=sys.stderr)
            return 1

    started = time.time()
    plan, result = mix_two_tracks(
        args.inputs[0],
        args.inputs[1],
        args.output,
        bars=args.bars,
        preview_seconds=args.preview,
        cache_dir=args.cache_dir,
        beat_backend=args.beats,
        bass_swap=not args.no_bass_swap,
        key_match=not args.no_key_shift,
        smart=not args.simple,
        style=args.style,
        vocals=args.vocals,
        plot=args.plot,
    )

    print(f"Track A   : {Path(args.inputs[0]).name}  ({plan.bpm_a:.2f} BPM)")
    print(f"Track B   : {Path(args.inputs[1]).name}  ({plan.bpm_b:.2f} BPM)")
    print(f"Style     : {plan.style}  ({plan.style_reason})")
    overlap = plan.style in ("blend", "filter")
    if overlap:
        print(f"Mix out   : A from {_fmt_time(plan.a_start)} to {_fmt_time(plan.a_end)}  ({plan.bars_a} bars)")
        print(f"Mix in    : B from {_fmt_time(plan.b_start)}  ({plan.bars_b} bars)")
    else:
        print(f"Mix out   : A stops at {_fmt_time(plan.a_start)}")
        print(f"Mix in    : B starts from {_fmt_time(plan.b_start)}")
    if not overlap:
        pass
    elif plan.beat_matched:
        tempo_a = 240.0 * plan.bars_a / (plan.a_end - plan.a_start)
        tempo_b = 240.0 * plan.bars_b / (plan.b_end - plan.b_start)
        change = (tempo_a / tempo_b - 1.0) * 100.0
        print(f"Tempo ramp: {tempo_a:.1f} -> {tempo_b:.1f} BPM  (B starts at {change:+.2f} % speed, ends at 0 %)")
        print(f"Stretch   : {result.stretch_engine}")
        for name, steady in (("A", plan.steady_a), ("B", plan.steady_b)):
            if not steady:
                print(
                    f"WARNING   : the bar lines found in track {name} are uneven here. Either the"
                    " tempo really changes, or the beat tracker made mistakes."
                    " Listen for wobble or a galloping beat. beat_this usually fixes this."
                )
    else:
        print("Beat match: NO - tempos are more than 8 % apart, plain crossfade used")
    if plan.key_a:
        played = "" if plan.pitch_shift_b == 0 else f" -> played as {plan.key_b_played}"
        print(
            f"Key       : A {plan.key_a}, B {plan.key_b}{played}"
            f"  (match {plan.key_score:.1f} of 1.0; {plan.key_note})"
        )
    print(f"Loudness  : B changed by {result.gain_b_db:+.1f} dB, whole mix by {result.master_gain_db:+.1f} dB")
    if overlap and plan.score is not None:
        parts = "  ".join(f"{name} {value:.2f}" for name, value in plan.score_parts.items())
        print(f"Score     : {plan.score:.2f}   ({parts})")
    if overlap and args.explain and plan.alternatives:
        print()
        print("Best candidates (the first one is used):")
        print("   A from   B from  bars  score   flow  clean   keep  phrase  vocal")
        for c in plan.alternatives:
            p = c["parts"]
            print(
                f"  {_fmt_time(c['a_start']):>7}  {_fmt_time(c['b_start']):>7}  {c['bars']:4d}"
                f"  {c['score']:5.2f}  {p['flow']:5.2f}  {p['clean']:5.2f}  {p['keep']:5.2f}  {p['phrase']:6.2f}"
                + (f"  {p['vocal']:5.2f}" if "vocal" in p else "      -")
            )
    if plan.vocal_clash is not None:
        if result.vocal_swap is not None:
            action = f"voice changes from A to B at {_fmt_time(result.vocal_swap)} in the output file"
        else:
            action = "nothing to do"
        print(f"Vocals    : both sing in {plan.vocal_clash * 100:.0f} % of the overlap; {action}")
    print()
    if result.bass_swap is not None:
        print(f"Bass swap : at {_fmt_time(result.bass_swap)} in the output file")
    print(
        f"In {args.output}: transition from {_fmt_time(result.transition_start)}"
        f" to {_fmt_time(result.transition_end)}"
    )
    if args.plot:
        print(f"Plot      : {args.plot}")
    print(f"Time: {time.time() - started:.1f} s")
    return 0


def _cmd_playlist(args: argparse.Namespace) -> int:
    from .audio_io import find_audio_files
    from .playlist import mix_playlist

    if args.plot is not None:
        print("--plot works with two songs only.", file=sys.stderr)
        return 1
    if args.preview is not None:
        print("--preview works with two songs only.", file=sys.stderr)
        return 1
    try:
        files = find_audio_files(args.inputs)
    except FileNotFoundError as exc:
        print(exc, file=sys.stderr)
        return 1
    if len(files) < 2:
        print("A playlist needs at least two songs.", file=sys.stderr)
        return 1

    started = time.time()
    try:
        report = mix_playlist(
            files,
            args.output,
            bars=args.bars,
            cache_dir=args.cache_dir,
            beat_backend=args.beats,
            bass_swap=not args.no_bass_swap,
            key_match=not args.no_key_shift,
            smart=not args.simple,
            style=args.style,
            vocals=args.vocals,
            order=args.order,
            limit=args.limit,
            progress=print,
        )
    except ValueError as exc:
        print(exc, file=sys.stderr)
        return 1

    styles: dict[str, int] = {}
    for entry in report["entries"][1:]:
        styles[entry["style"]] = styles.get(entry["style"], 0) + 1
    summary = ", ".join(f"{count} {name}" for name, count in sorted(styles.items()))
    print()
    print(f"Set       : {len(report['entries'])} songs, {_fmt_time(report['duration'])}  ({summary})")
    print(f"Audio     : {report['out']}")
    print(f"Track list: {report['tracklist']}")
    print(f"Time: {time.time() - started:.1f} s")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="automix", description="DJ-style automatic song transitions."
    )
    sub = parser.add_subparsers(dest="command", required=True)

    analyze = sub.add_parser("analyze", help="Analyze tracks: BPM, key, downbeats, phrases, energy.")
    analyze.add_argument("inputs", nargs="+", help="Audio files or folders.")
    analyze.add_argument(
        "--beats",
        choices=["auto", "beat_this", "librosa"],
        default="auto",
        help="Beat tracker. 'auto' uses beat_this when installed, else librosa.",
    )
    analyze.add_argument("--cache-dir", default="automix_cache", help="Where JSON results are saved.")
    analyze.add_argument("--no-cache", action="store_true", help="Analyze again even if a cached result exists.")
    analyze.add_argument(
        "--vocals",
        action="store_true",
        help="Also separate the vocals with Demucs and save them (slow).",
    )
    analyze.set_defaults(func=_cmd_analyze)

    common = argparse.ArgumentParser(add_help=False)
    common.add_argument(
        "inputs", nargs="+", help="Two audio files (A then B), or a folder / more files for a playlist."
    )
    common.add_argument("-o", "--output", default="out.wav", help="Output WAV file.")
    common.add_argument("--bars", type=int, default=16, help="Longest transition in bars (default 16).")
    common.add_argument(
        "--preview",
        type=float,
        default=None,
        metavar="SECONDS",
        help="Only write the transition plus this many seconds before and after.",
    )
    common.add_argument(
        "--no-bass-swap",
        action="store_true",
        help="Fade the bass slowly like the other bands (to compare by ear).",
    )
    common.add_argument(
        "--no-key-shift",
        action="store_true",
        help="Never pitch-shift track B to match the key of track A.",
    )
    common.add_argument(
        "--style",
        choices=["auto", "blend", "filter", "cut", "echo"],
        default="auto",
        help="Transition style. 'auto' picks one from tempo, energy and key.",
    )
    common.add_argument(
        "--vocals",
        action="store_true",
        help="Vocal-aware mixing with Demucs stems (slow the first time for each song).",
    )
    common.add_argument(
        "--simple",
        action="store_true",
        help="Use the simple rule (end of A, start of B) and not the smart planner.",
    )
    common.add_argument(
        "--plot",
        default=None,
        metavar="PNG",
        help="Two songs: also save a debug picture of the transition (needs matplotlib).",
    )
    common.add_argument(
        "--explain", action="store_true", help="Show the best candidates and their scores."
    )
    common.add_argument("--beats", choices=["auto", "beat_this", "librosa"], default="auto")
    common.add_argument("--cache-dir", default="automix_cache", help="Where JSON results are saved.")
    common.add_argument(
        "--order",
        choices=["given", "smart"],
        default="given",
        help="Playlist: 'given' keeps the order, 'smart' puts easy-to-mix songs next to each other.",
    )
    common.add_argument(
        "--limit", type=int, default=None, metavar="N", help="Playlist: use only the first N songs."
    )
    mix = sub.add_parser(
        "mix", parents=[common], help="Mix two songs (or a playlist) with beat-matched transitions."
    )
    mix.set_defaults(func=_cmd_mix)
    playlist = sub.add_parser(
        "playlist", parents=[common], help="Mix a folder or a list of songs into one continuous file."
    )
    playlist.set_defaults(func=_cmd_playlist)
    return parser


def main(argv: list[str] | None = None) -> int:
    # File names can contain characters the Windows console cannot print.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(errors="replace")
    # "automix a.mp3 b.mp3 -o out.wav" is a short form of "automix mix ...".
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv and argv[0] not in ("analyze", "mix", "playlist", "-h", "--help"):
        argv.insert(0, "mix")
    args = build_parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
