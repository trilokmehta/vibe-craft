# automix

Open-source DJ-style song transitions. Give it two or more songs and it will
(when finished) beat-match them, match the key, swap the bass on a downbeat and
crossfade on phrase lines, like a human DJ.

**Status:** Milestone 9 of 10. The analyzer works, and two tracks can be mixed
with a beat-matched 16-bar transition, a tempo ramp, a bass swap and key matching. Transition points are chosen by
scoring phrases and energy, there are four transition styles, and with `--vocals` two singers never overlap. A folder of songs becomes one
continuous mix.

## Folder structure

```
automix/
  pyproject.toml              package settings, makes the "automix" command
  requirements.txt            core libraries (all install on Windows)
  requirements-optional.txt   beat_this (better beats, needs PyTorch)
  src/automix/
    cli.py                    command line
    mix.py                    two songs: analyze -> plan -> render -> write WAV
    playlist.py               many songs -> one continuous WAV + track list
    camelot.py                Camelot wheel: which keys fit, which pitch shift helps
    audio_io.py               loading audio files
    analyzer/
      core.py                 runs all steps, saves the JSON cache
      beats.py                BPM, beats, downbeats
      key.py                  key + Camelot code
      structure.py            8-bar phrases and section labels
      energy.py               LUFS loudness and energy curve
      stems.py                vocal stem with Demucs, singing per bar
    planner/
      simple.py               tempo ramp, bar grid, key shift; simple rule for the points
      smart.py                scores many candidate points and picks the best
      style.py                the four styles and the automatic choice
      vocals.py               vocal clash score and vocal swap point
      order.py                track order for a playlist (--order smart)
    renderer/
      stretch.py              variable-speed time-stretch (Rubber Band, or built-in fallback)
      eq.py                   3-band split, fade curves, bass swap, filter sweep
      crossfade.py            puts the transition together
      limiter.py              look-ahead limiter for the whole set
  tests/                      tests that use a computer-made track
```

## Setup on Windows 11 (PowerShell)

WSL2 is **not** needed for anything in Milestones 1 and 2.

### 1. Python 3.11 and FFmpeg

```powershell
winget install Python.Python.3.11
winget install Gyan.FFmpeg
```

Close and reopen PowerShell after this, so the new PATH is loaded. Check:

```powershell
python --version
ffmpeg -version
```

MP3, WAV, FLAC and OGG are read without FFmpeg. FFmpeg is needed for M4A/AAC.

### 2. Rubber Band CLI (recommended from Milestone 3)

Without it the mix still works, but uses a built-in lower-quality stretch
(drums sound a little smeared).

1. Download the Windows command-line build from <https://breakfastquay.com/rubberband/>.
2. Unzip it to `C:\Tools\rubberband` (so that `C:\Tools\rubberband\rubberband.exe` exists).
3. Add the folder to your PATH and reopen PowerShell:

```powershell
[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path", "User") + ";C:\Tools\rubberband", "User")
```

```powershell
rubberband --version
```

### 3. Install automix

Run these inside the `automix` folder:

```powershell
python -m venv .venv
.\.venv\Scripts\Activate.ps1
pip install -r requirements.txt
pip install -e .
```

If PowerShell blocks `Activate.ps1`, run this once and try again:

```powershell
Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
```

### 4. Optional: vocal-aware mixing with Demucs

```powershell
pip install torch torchaudio --index-url https://download.pytorch.org/whl/cpu
pip install demucs
```

The first run downloads the `htdemucs` model (needs internet once).

### 5. Optional: better beats with beat_this

```powershell
pip install torch --index-url https://download.pytorch.org/whl/cpu
pip install -r requirements-optional.txt
```

The first run downloads the model file from the authors' server (JKU Linz),
so it needs internet once. If that download fails, the analyzer prints a
warning and uses librosa. Songs analyzed before beat_this worked keep their old
result in the cache; run `automix analyze <songs> --no-cache` once to redo them.

## Use

```powershell
automix analyze "C:\Music\song.mp3"
automix analyze "C:\Music\folder" --beats librosa
```

It prints BPM, key, loudness, downbeats and a phrase table, and saves everything
to `automix_cache\<song>.<id>.json`. A second run loads the JSON (fast). Use
`--no-cache` to analyze again.

Mix two tracks:

```powershell
automix a.mp3 b.mp3 -o out.wav
automix a.mp3 b.mp3 -o check.wav --preview 20
```

`--preview 20` writes only the transition plus 20 seconds before and after, so
you can check it quickly. `--bars 8` makes a shorter transition.
`--no-bass-swap` fades the bass slowly like the other bands, to compare by ear.
`--no-key-shift` never changes the pitch of track B.
`--style blend|filter|cut|echo` forces a style (default: `auto`).
A folder, or more than two files, makes a playlist (see below).
`--vocals` turns on vocal-aware mixing (slow the first time for each song).
`automix analyze <songs> --vocals` separates the vocals ahead of time.
`--explain` shows the five best candidates and their scores.
`--simple` uses the old rule (end of A, start of B) to compare by ear.
`--bars` is now the longest transition allowed (8, 16 or 32 are tried).

Run the tests (no music needed):

```powershell
pytest
```

## How the mix works (Milestones 3 and 4)

1. **Mix-out point:** the last phrase line in track A that still has 16 bars of
   music after it (silent endings are skipped).
2. **Mix-in point:** the first full 8-bar phrase of track B.
3. **Beat matching with tempo ramp:** the transition starts at A's tempo and
   ends at B's tempo. The speed changes a little on every bar, and both tracks
   follow it, so their bar lines always fall together. Before the transition A
   is untouched, after it B is untouched. If the tempos are more than 8 %
   apart (also after trying half/double time), nothing is stretched and you
   get a plain crossfade.
4. **Steady grid:** the beat tracker's bar lines are a little noisy. Inside the
   transition they are replaced by an evenly spaced grid when most of them
   agree with one. If not, the tool prints a WARNING and uses them as measured.
5. **Loudness:** B is brought to A's LUFS level.
6. **3-band EQ:** both tracks are split into low (< 200 Hz), mid and high
   (> 2.5 kHz). Mid and high use a slow equal-power crossfade.
7. **Bass swap:** A keeps the bass for the first 8 bars. On the bar line in the
   middle, A's bass goes off and B's bass comes on (30 ms fade, no click).
   So only one kick drum and one bassline play at any time.

To read: Rubber Band's time map option is described in `rubberband --help`
(`--timemap`). Phase vocoder: Flanagan & Golden (1966); Dolson, "The Phase
Vocoder: A Tutorial" (1986). The bass-swap rule is standard DJ practice; see
Vande Veire & De Bie (2018), section on "EQ and crossfading".

## Playlist mode (Milestone 9)

```powershell
automix "C:\Music\folder" -o set.wav
automix a.mp3 b.mp3 c.mp3 d.mp3 -o set.wav --order smart
automix "C:\Music\folder" -o set.wav --limit 10 --style blend
```

A folder is read in file-name order. All options of two-song mixing work
here too, except `--preview`. New options: `--order given|smart` and
`--limit N` (use only the first N songs). Two files are a normal two-song mix;
`automix playlist ...` always makes a playlist.

How the set is built, one pair at a time:

1. **Order.** `given` keeps your order. `smart` starts with the first song and
   always plays next the unused song that is easiest to mix: tempo within 8 %
   (half and double time count) and a fitting Camelot key. It is a greedy
   order, not the best possible one.
2. **Loudness.** Every song is brought to the median loudness of the set
   (at most 12 dB up or down).
3. **Transitions.** The same planner as before, with two additions. A
   transition may not start in the part of a song that the transition before
   it already used, and a song plays at least 8 bars alone between two
   transitions. If a song has no room left for an overlap, a **cut** is used.
4. **Keys carry on.** If a song was pitch-shifted to fit the one before it, it
   stays shifted, and the next transition starts from that new key.
5. **Nothing is played twice.** Each song is written from where the previous
   transition ended to where the next one starts.
6. **Limiter.** One look-ahead limiter (ceiling 0.98) works on the whole set,
   so peaks are turned down only where they happen, and there is no loudness
   jump between transitions. The WAV is written piece by piece, so a long set
   does not have to fit in memory.
7. **Track list.** `set.tracklist.txt` is written next to the WAV: when each
   song is first heard, how it was mixed in, and warnings about uneven bar
   lines. Open it with a text editor that reads UTF-8.

`--vocals` works in playlists too, but every song has to go through Demucs
first (one to three minutes each, once; the result is cached).

To read: Mixxx AutoDJ keeps the same idea of one transition per pair with
fade-in and fade-out regions (manual, chapter "Auto DJ"). Limiter design:
Zölzer, "DAFX: Digital Audio Effects", chapter on dynamic range control.

## Vocal-aware mixing (Milestone 8)

With `--vocals`, Demucs splits each song and the vocal stem is saved as a FLAC
file in the cache folder. On a CPU this often takes one to three minutes per
song, once. From the stem we get one number per bar: 0 = nobody sings,
1 = full singing.

1. **Planning.** A new score part, `vocal`, is 1 when the two tracks never
   sing at the same time during the overlap and 0 when they always do. It
   counts for 25 % of the total score. So the planner prefers "A sings over
   B's intro" or "B sings over A's instrumental outro".
2. **Vocal swap.** If both still sing in the overlap, only one voice is heard
   at a time: A's voice until a bar line near the middle, then B's voice. The
   bar line is chosen where the least singing gets cut (a pause, if there is
   one). The voices fade over 0.25 s. The instruments are mixed as before.

Without `--vocals` nothing changes, and no stems are needed.

To read: Défossez, "Hybrid Transformers for Music Source Separation" (ICASSP
2023), the Demucs paper; code at <https://github.com/adefossez/demucs> (MIT).
Apple's AutoMix and djay's "Neural Mix" use the same idea of mixing stems.

## Transition styles (Milestone 7)

| Style | What you hear | Beat-matched |
|---|---|---|
| blend | Long mix, each band fades on its own, bass swaps on a bar line | yes |
| filter | A rising high-pass filter removes A's bass, then its body; B comes in under it | yes |
| cut | A stops on a phrase line, B starts on its "one" (10 ms fade) | no |
| echo | Like cut, and A's last beat repeats 8 times, each one quieter, without bass | no |

How `auto` chooses:

1. Tempos more than 8 % apart -> **echo** (the beats cannot be matched, and the
   echo hides the tempo jump).
2. One track is calm during the overlap ("clean" score 0.5 or more) -> **blend**.
3. Both tracks busy and the keys clash (key score under 0.5) -> **cut**.
4. Both tracks busy and the keys fit -> **filter**.

For cut and echo there is no overlap, so the points are chosen differently:
A stops on a phrase line near its end, and B starts on the phrase line where
its energy is closest to A's energy before the cut. B is never pitch-shifted
for these two styles.

To read: Mixxx has the same idea of several AutoDJ transition modes
("Full Intro + Outro", "Fade At Outro Start", "Skip Silence"; see the Mixxx
manual, chapter "Auto DJ"). For the filter and echo effects see the Mixxx
effects code (`src/effects/backends/builtin/`, GPL: read, do not copy).

## Smart transition points (Milestone 6)

The planner tries every phrase line in the second half of A as mix-out point,
every phrase line in the first 40 % of B as mix-in point, and lengths of 8, 16
and 32 bars. Each candidate gets a score:

| Part | Weight | Good when |
|---|---|---|
| flow | 0.25 | energy after the transition (B) is close to energy before it (A); a drop is punished more than a rise |
| clean | 0.25 | the two tracks are not both at full power while they overlap (A's outro over B's intro is best) |
| keep | 0.30 | little music is thrown away (cut end of A + skipped start of B; 32 bars lost = 0) |
| phrase | 0.20 | start and end of the transition are phrase lines in both tracks |
| length | +-0.10 | long transition when it is clean, short one when both tracks are busy |

Energy per bar is measured against the loud parts of the same track: 0 means
12 dB or more below them, 1 means as loud as them.

The weights are a first guess, in `planner/smart.py`. Run with `--explain`,
listen, and change them.

To read: Vande Veire & De Bie (2018) choose cue points from structure and
drop positions; Bittner et al. (ISMIR 2017) score transitions; Schwarz &
Fourer, "Methods and Datasets for DJ-Mix Reverse Engineering" (CMMR 2019)
measure what real DJs do.

## Key matching (Milestone 5)

Every key gets a Camelot code: a number 1-12 and a letter (A = minor,
B = major). Keys next to each other on the Camelot wheel share most of their
notes, so melodies and chords do not clash when both songs play together.

| From A to B | Example | Score |
|---|---|---|
| Same key | 8A -> 8A | 1.0 |
| Relative major/minor | 8A -> 8B | 0.9 |
| One step, same letter | 8A -> 7A or 9A | 0.9 |
| One step, other letter | 8A -> 9B | 0.6 |
| Two steps up ("energy boost") | 8A -> 10A | 0.5 |
| One semitone up | 8A -> 3A | 0.4 |
| Anything else | 8A -> 2A | 0.1 (clash) |

If the keys do not fit, the planner checks whether moving B's pitch by 1 or 2
semitones up or down gives a clearly better score (at least 0.2 better; each
semitone costs 0.05). If yes, **all of B** is pitch-shifted, so its key does
not jump after the transition. Tempo is not changed by this.

The shift is only done when the key detector is sure about both songs
(confidence 0.1 or more). The confidence says how clearly the detected key
beats the keys that would clash with it. A wrong shift is worse than no shift.

To read: the Camelot wheel comes from Mark Davis ("Harmonic Mixing", Camelot
Sound). Open-source key detection for DJs: Ibrahim Sha'ath, "Estimation of key
in digital music recordings" (MSc thesis, 2011) and his KeyFinder code
(libKeyFinder, GPL-3.0; read for ideas). Key-detection research for dance
music: Faraldo et al., "Key Estimation in Electronic Dance Music", ECIR 2016.

## How the analyzer works

| Part | Method | Read more |
|---|---|---|
| Beats | `beat_this` neural network when installed; else librosa's dynamic-programming tracker | Foscarin et al., "Beat This!", ISMIR 2024; Ellis, "Beat Tracking by Dynamic Programming", 2007 |
| Downbeats | From `beat_this`. With librosa: a guess from bass hits + chord changes, assumes 4/4 | Böck et al., "Joint Beat and Downbeat Tracking with RNNs", ISMIR 2016 |
| Key | Chroma profile compared with 24 Krumhansl-Kessler key profiles | Krumhansl, "Cognitive Foundations of Musical Pitch", 1990 |
| Phrases | 8-bar grid placed on the biggest changes in sound (novelty) | Foote, "Automatic Audio Segmentation Using a Measure of Audio Novelty", 2000 |
| Loudness | Integrated LUFS | ITU-R BS.1770-4; EBU R 128 |

For the whole idea of automatic DJ mixing, good starting points are:

- Mixxx AutoDJ source code: `src/library/autodj/autodjprocessor.cpp` in <https://github.com/mixxxdj/mixxx> (GPL-2.0+; read for ideas, do not copy into non-GPL code).
- Vande Veire & De Bie, "From raw audio to a seamless mix: creating an automated DJ system for Drum and Bass", EURASIP J. Audio, Speech, Music Process., 2018.
- Bittner et al., "Automatic Playlist Sequencing and Transitions", ISMIR 2017.

## Known limits (Milestone 2)

- The librosa beat tracker can report half or double tempo (e.g. 70 for 140 BPM).
- The librosa downbeat guess is right most of the time on 4/4 music with a clear
  kick, and can be 1-3 beats off on other music. `beat_this` is much better.
- Section labels come from loudness only. "chorus" means "loud part" (chorus or drop).
- Key detection is often confused between a key and its relative (e.g. C major
  and A minor, 8B and 8A). These two are compatible for mixing, so it rarely hurts.
- Key detection has not been measured against songs with known keys yet. Check
  a few of your songs by ear before you trust the pitch shift.
- Playlist order is greedy and only looks at tempo and key, not at energy or mood.
- Each song is loaded and pitch-shifted completely when its turn comes. For long
  playlists this takes time, but memory use stays at about two songs.
- A pitch shift of 2 semitones can make voices sound slightly unnatural,
  more so without Rubber Band.

## Licenses of the tools used

| Tool | License | Note |
|---|---|---|
| numpy, scipy, soundfile | BSD | |
| librosa, pyrubberband | ISC | |
| pyloudnorm, beat_this, demucs | MIT | |
| Rubber Band CLI | GPL-2.0+ | Called as a separate program, not linked into this code |
| FFmpeg | LGPL/GPL | Called as a separate program |
| pedalboard (maybe later) | GPL-3.0 | Importing it would make this project GPL-3.0 |
| madmom | BSD code, **non-commercial** model files; hard to install on Windows | Not used |
| essentia | AGPL-3.0; no Windows wheels (WSL2 only) | Not used |

This project has no license file yet. MIT keeps things simple as long as GPL
libraries such as pedalboard are not imported; choose GPL-3.0 if you want them.
