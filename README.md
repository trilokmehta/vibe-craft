# Vibe Craft

An AI DJ for Windows that plays your own music library and mixes song to song like a DJ does.

It analyses every song for tempo, key and energy, picks what to play next, and blends the songs together. When it can, it does this **layer by layer**: an AI model splits each song into vocals, melody, music, bass and drums, so the transition brings in the next song's beats first, then swaps the bass, crosses over the instruments and leaves the vocals for last.

Built in Rust with [macroquad](https://github.com/not-fl3/macroquad), [cpal](https://github.com/RustAudio/cpal), [symphonia](https://github.com/pdeljanov/Symphonia) and [ONNX Runtime](https://onnxruntime.ai/).

## Download

Get the latest `vibe-craft.exe` from the [Releases page](https://github.com/trilokmehta/vibe-craft/releases). It's a single file, so no installer is needed.

**First run:** Windows may show "Windows protected your PC" because the app isn't code-signed yet. Click **More info**, then **Run anyway**.

The app updates itself: when a newer release is out, an **Update** button appears in the header.

## Features

- **Your music:** choose any folder with MP3, WAV, FLAC or OGG files. Everything is analysed on your computer.
- **Auto DJ:** picks the next song by tempo, key, energy, mood and what you like or skip.
- **AutoMix:** finds the best place to leave the playing song and enter the next one: phrase ends, instrumental stretches, build-ups.
- **AI layer separation:** HT-Demucs splits songs into Vocals, Melody (guitar + piano), Music (synths, strings, pads), Bass and Drums. You can turn each layer up or down, or use the Instrumental and Acapella presets.
- **Layered transitions:** beats → bass swap → instruments and melody → vocals. The order adapts when the next song opens with instruments, when the melodies' notes clash, or when tempos can't be matched. A safeguard prevents quiet gaps.
- **Beat matching:** songs within about 16% of each other's tempo are time-stretched (pitch kept) and lined up on the beat.
- **Layer timeline:** the Smart DJ screen shows each layer fading out and in while a mix runs.
- **Pro decks:** two virtual decks with cues, loops, EQ and a crossfader, plus an optional second audio output for headphone cueing.

## Controls

| Key | Action |
|---|---|
| Space | Play / pause |
| Left / Right | Previous / next song |
| M | Mix to the next song now |
| D | Auto DJ on / off |
| 1–4 | Now Playing, Smart DJ, Queue, Library |
| Up / Down | Volume |
| L | Like the current song |

## Files the app stores

Everything lives in `%LOCALAPPDATA%\VibeCraft`:

| Folder | What | Size |
|---|---|---|
| `models` | AI separation model, downloaded once | ~136 MB (+ ~136 MB for the GPU split) |
| `runtime` | ONNX Runtime for GPU separation | ~16 MB |
| `stems` | Separated layers, one file per song | ~64 MB per song, capped at 3 GB |

Delete the folder to reset. The app downloads what it needs again.

## Build from source

Requires Rust (stable, MSVC toolchain) on Windows.

```bash
cd build-a-smart-ai/app
cargo build --release
```

The exe is written to `build-a-smart-ai/app/target/release/vibe-craft.exe`. Run `cargo test --release` for the unit tests.

### Test and measurement commands

| Command | What it does |
|---|---|
| `vibe-craft.exe --sep-test <song> [out dir]` | Separates one song, writes each layer as WAV, prints timing and device |
| `vibe-craft.exe --layer-test [pairs]` | Renders real transitions layered vs. old curves into `layer_test/`, with a `report.md` of overlaps and loudness |
| `vibe-craft.exe --vibe-test [n]` | Compares mix-point choices with and without layer data |
| `vibe-craft.exe --lib-stats` | Lists tempo and beat confidence for every song in the library |
| `vibe-craft.exe --screenshot <file.png>` | Renders the UI with demo songs and saves a screenshot |

Environment variables: `VC_CPU=1` forces CPU separation, `VC_DEBUG=1` prints loudness curves in `--layer-test`.

## Licences

- Vibe Craft source: see the repository.
- HT-Demucs model: MIT (Meta / [facebookresearch/demucs](https://github.com/facebookresearch/demucs)), ONNX export by [StemSplitio](https://huggingface.co/StemSplitio/htdemucs-6s-onnx).
- ONNX Runtime: MIT (Microsoft). DirectML is the copy that ships with Windows.
