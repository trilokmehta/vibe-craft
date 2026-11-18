# Smart AI DJ - mechanics sheet

No original product exists to measure; every value is a design choice from the request (marked "estimated").

| Group | Value | Note |
|---|---|---|
| Analysis | tempo search 70-175 BPM, 0.01 BPM refinement | estimated |
| Analysis | onset hop 64 samples at 22050 Hz | estimated |
| Mixing | tempo-match limit +/-8% (half/double time allowed) | estimated |
| Mixing | phrase = 16 beats, bar = 4 beats | musical convention |
| Mixing | blend length 8/16/32 beats by intensity (calm moods min 16) | estimated |
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
