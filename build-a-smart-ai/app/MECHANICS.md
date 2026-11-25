# Smart AI DJ - mechanics sheet

No original product exists to measure; every value is a design choice from the request (marked "estimated").

| Group | Value | Note |
|---|---|---|
| Analysis | tempo search 70-175 BPM, 0.01 BPM refinement | estimated |
| Analysis | onset hop 64 samples at 22050 Hz | estimated |
| Mixing | tempo-match limit +/-8% (half/double time allowed) | estimated |
| Mixing | phrase = 16 beats, bar = 4 beats | musical convention |
| Mixing | blend length 8/16/32 beats by intensity (calm moods min 16), capped length also sets which shorter lengths AutoMix's point search is allowed to try and score against it | estimated |
| Mixing | EQ crossover 250 Hz / 2.5 kHz, bass swap at 50% of blend | estimated |
| Mixing | echo = dotted eighth, feedback 0.5, ping-pong | estimated |
| Mixing | tempo returns to normal after a mix at 0.4% per second | estimated |
| Mixing | Blend / Filter+echo glide the tempo during the mix (outgoing deck ramps to the tempo where the next song plays unstretched, both follow bar by bar), ported from automix | automix |
| Mixing | key shift: next song moved up to +-2 semitones when that lifts the Camelot match by 0.2 (0.05 per semitone), only when both key detections are sure (>= 0.05); kept for the rest of that song, tempo unchanged | automix |
| Queue | keeps 4 tracks, avoids last 8 played, penalises same artist | estimated |
| Mood energy targets | Chill .30, Party .85, Workout .80, Focus .42, Romantic .25, Discovery .55 | estimated |
| Controls | Space pause, Left/Right prev/next, M mix now, D auto DJ, 1-4 tabs, Up/Down volume, L like | estimated |

## Learned from the training DJ sets (training 1/2/3)

Retrained 2026-11-19 (`trainer match` -> `segments` -> `fit`, 40/702 training songs matched into the mix, 7 usable transitions):

| Habit seen in the real mixes | Value | Now in the engine |
|---|---|---|
| Time each song plays | median 39 s, p10/90 18/59 s (8-29% of the song, median 17%) | party pacing: may leave after 30% of a song (calm / 32-beat blends keep 55%) |
| Where songs are entered | median 15% in (p10/90 4/31%), 3/40 entered at the opening | entry points chosen past the intro (unchanged) |
| Tempo shift on a song | median 3.0%, p90 7.0%, up to 11.0% | beat-match allowed up to 16% (unchanged) |
| BPM step after shifting | median 1.2%, p90 2.2%, up to 2.9% | tempo weight raised 2.2 -> 3.0 (unchanged, consistent with the new sample) |
| Camelot distance | mostly 0-2 over the 7 transitions seen ([1,0,2,1,2,1,0]) | key weight 1.6 -> 0.9 (tempo is the main cue; key clashes are hidden with FX) |
| Groove similarity | | dance weight 0.5 -> 0.7 |
| Transition tricks | short punchy mixes, loops, echo / reverb throws | new styles: Loop roll (1 bar -> 1/8 then drop on the one), Reverb wash; party sets rotate Blend / Filter+Echo (Loop roll is switched off in `app.rs`, `LOOP_ROLL_AUTO`, because it sounded like a stuck loop), key clashes alternate Echo out / Reverb wash |

Note: this run's regression over the weights (`trainer fit`) only had 7 real transitions to learn from, too few to trust - it collapsed onto "dance-closeness only" (`learned (scaled): [0.00, 0.00, 0.00, 5.70]` vs the hand-tuned `[2.2, 1.6, 1.4, 0.5]`). The engine weights above were left as-is; only the timing/entry/tempo/Camelot habit numbers were refreshed from this run. Full output: `training 1/results/fit.txt`.

## Profile 3 (training 5, 2026-11-26)

Third mix (`training 5`, 20 of 27 songs located, 13 usable transitions; `trainer profile3` -> `training 5/results/profile3.txt`):

| Habit seen | Value | In the engine |
|---|---|---|
| Learned weights (tempo, key, energy, dance, low, mid, high balance) | raw [1.48, -0.38, 0.32, -0.53, -0.04, -0.03, -0.04]; scaled to 6: tempo 4.9, energy 1.1, rest ~0 | Profile 3 = tempo 4.9, key 0.3, energy 1.1, dance 0; band balance (low/mid/high closeness 0.3/0.3/0.1) added by hand because the fit found no real pull either way (13 cases) |
| Mix balance | songs play at ~56% low / 42% mid / 2% high; across a hand-over lows drop ~3%, mids rise ~3% | not modelled beyond the closeness terms |
| Where the DJ comes in | median 3.2 bars after the melody starts, only 2/20 before it; ~18 s after the first sound | Profile 3 penalises entry points more than a bar before the melody onset |
| Where the DJ leaves | 20/20 before the outro, median ~77 bars early (songs play ~40-60 s) | unchanged |

Profile 3 retrained 2026-11-26 with a second mix (`training 6`, 12 of 18 songs located, 8 transitions; `training 6/results/profile3.txt`): learned tempo 5.9 / energy 0 / dance 0.1 (scaled to 6), bands again ~0. Combined with training 5 by transition count (13 + 8): **tempo 5.3, energy 0.65, key 0.3 (hand), dance 0**; band terms unchanged. The DJ again came in after the melody (11 of 12; median 8.6 bars after it, ~25 s after first sound) and left every song before its outro (median ~107 s of the song unplayed). Mix balance 63% low / 35% mid / 2% high.

## Profiles 4-6 (2026-11-26)

One profile per training mix (`trainer profile3`; weights scaled to total 6; key 0.3 hand-set on 4; band terms 0 because the fit never found a pull):

| Profile | Mix | Located / transitions | tempo | key | dance | energy | Notes |
|---|---|---|---|---|---|---|---|
| 4 | training 7 | 33 of 49 / 11 | 5.2 | 0.3 | 0 | 0.8 | DJ in 6.9 bars after melody, left 24/24 before outro |
| 5 | training 8 | 25 of 31 / 16 | 1.5 | 3.4 | 0 | 1.1 | key-led; mix 55/42/3 low/mid/high |
| 6 | training 9 | 22 of 24 / 16 | 3.1 | 2.6 | 0.1 | 0 | tempo and key about equal; DJ in 9.6 bars after melody |

Profiles 3-6 all use the "enter after the melody" rule (54 of 57 songs across the mixes did).

## Not turned into profiles (2026-11-26)

- `training 10` (club / Jersey-club edits): 20 of 30 songs located but only 3 usable transitions, so the weight fit is noise (raw [1.7, -0.2, 1.3, -0.9, 1.4, 1.5, -0.1]). Habits were consistent with the other mixes: entered after the melody 15/15 (median 14 bars after), left before the outro 15/15, mix balance 79% low / 20% mid / 1% high.
- `training 11` (DJ NYK remixes): only 2 of 6 songs located, 0 transitions - the remixed versions could not be downloaded as separate tracks.

## Merged DJ profile (all training mixes)
Profiles 1-6 are replaced by one profile fitted on training 1-11 pooled (156 transitions, 244 songs;
`trainer pooled`, output in `training_all.txt`): tempo 3.70, key 1.16, energy 0.54, dance 0.17, high-band balance 0.43.
Entry comes after the melody (225 of 244 songs did). Researched pro-DJ rules added to AutoMix:
16/32-bar phrase starts score higher than 8-bar ones, and overlaps where both songs carry lead vocals are penalised.
The engine's bass swap (one track owns the lows) was already in place.
