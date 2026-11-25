//! DJ decision engine, recommendation engine and listening-history
//! personalisation.
use crate::tracks::{camelot, camelot_str, Track};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mood {
    Chill,
    Party,
    Workout,
    Focus,
    Romantic,
    Discovery,
}

pub const MOODS: [Mood; 6] = [Mood::Chill, Mood::Party, Mood::Workout, Mood::Focus, Mood::Romantic, Mood::Discovery];

impl Mood {
    pub fn name(&self) -> &'static str {
        match self {
            Mood::Chill => "Chill",
            Mood::Party => "Party",
            Mood::Workout => "Workout",
            Mood::Focus => "Focus",
            Mood::Romantic => "Romantic",
            Mood::Discovery => "Discovery",
        }
    }
    pub fn base_energy(&self) -> f32 {
        match self {
            Mood::Chill => 0.30,
            Mood::Party => 0.85,
            Mood::Workout => 0.80,
            Mood::Focus => 0.42,
            Mood::Romantic => 0.25,
            Mood::Discovery => 0.55,
        }
    }
    /// How far energy may move from one track to the next.
    pub fn max_step(&self) -> f32 {
        match self {
            Mood::Focus => 0.08,
            Mood::Workout => 0.10,
            Mood::Chill | Mood::Romantic => 0.12,
            _ => 0.18,
        }
    }
    pub fn calm(&self) -> bool {
        matches!(self, Mood::Chill | Mood::Focus | Mood::Romantic)
    }
    fn tag_match(&self, t: &Track) -> f32 {
        let g = t.genre.as_str();
        let good: &[&str] = match self {
            Mood::Chill => &["Chill", "Lo-fi", "Romantic", "Focus"],
            Mood::Party => &["House", "Techno", "Pop", "Workout", "Synthwave"],
            Mood::Workout => &["Workout", "Techno", "House", "Hip-Hop", "Pop"],
            Mood::Focus => &["Focus", "Lo-fi", "Chill"],
            Mood::Romantic => &["Romantic", "Chill", "Pop"],
            Mood::Discovery => &[],
        };
        if good.contains(&g) {
            1.0
        } else if *self == Mood::Discovery || g == "Unknown" {
            0.7
        } else {
            0.35
        }
    }
}

/// Learned transition weights `score_candidate` uses when picking the next track.
/// One merged profile, fitted on every training mix (training 1 to 11) pooled together.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TransitionProfile {
    Merged,
}

impl TransitionProfile {
    pub fn name(&self) -> &'static str {
        "Merged DJ profile"
    }
    /// (tempo-fit, key-fit, dance-closeness, energy-closeness)
    pub fn weights(&self) -> (f32, f32, f32, f32) {
        MERGED_WEIGHTS
    }
    /// Pro DJs (and the training DJs, nearly every song) bring the next song in after its melody starts.
    pub fn after_melody(&self) -> bool {
        true
    }
    /// (low, mid, high) balance-closeness weights.
    pub fn band_weights(&self) -> (f32, f32, f32) {
        MERGED_BANDS
    }
}

/// Pooled fit over all training mixes (see training_all.txt).
/// 156 real transitions from 244 songs; fits them better than any single-mix profile did (loglik -2.46 vs -2.54..-2.91).
const MERGED_WEIGHTS: (f32, f32, f32, f32) = (3.70, 1.16, 0.17, 0.54);
const MERGED_BANDS: (f32, f32, f32) = (0.0, 0.0, 0.43);

#[derive(Clone)]
pub struct Settings {
    pub auto: bool,
    pub mood: Mood,
    pub energy: f32,
    pub genre: Option<String>,
    pub intensity: f32,
    pub discovery: f32,
    /// Layer build: the next song enters in stages (music, melody, drums + bass, vocals).
    pub layer_build: bool,
    pub profile: TransitionProfile,
    /// Transition feel: DJ = club pacing (short, punchy, songs left early like the training DJs);
    /// Smooth = long calm blends that let songs play most of the way.
    pub smooth: bool,
}

impl Settings {
    pub fn new() -> Settings {
        Settings { auto: true, mood: Mood::Party, energy: 0.5, genre: None, intensity: 0.5, discovery: 0.35, layer_build: false, profile: TransitionProfile::Merged, smooth: false }
    }
    pub fn target_energy(&self) -> f32 {
        (self.mood.base_energy() + (self.energy - 0.5) * 0.8).clamp(0.05, 1.0)
    }
}

#[derive(Default)]
pub struct Profile {
    pub genre_aff: HashMap<String, f32>,
    pub artist_aff: HashMap<String, f32>,
    pub track_aff: Vec<f32>,
    pub plays: Vec<u32>,
    pub skips: Vec<u32>,
    pub liked: Vec<bool>,
    pub disliked: Vec<bool>,
    pub history: Vec<usize>,
    pub listened_secs: f32,
    pub sessions: u32,
}

impl Profile {
    pub fn new(n: usize) -> Profile {
        Profile {
            track_aff: vec![0.0; n],
            plays: vec![0; n],
            skips: vec![0; n],
            liked: vec![false; n],
            disliked: vec![false; n],
            ..Default::default()
        }
    }
    fn bump(&mut self, t: &Track, d: f32) {
        self.track_aff[t.id] = (self.track_aff[t.id] + d).clamp(-2.0, 2.0);
        let g = self.genre_aff.entry(t.genre.clone()).or_insert(0.0);
        *g = (*g + d * 0.5).clamp(-2.0, 2.0);
        let a = self.artist_aff.entry(t.artist.clone()).or_insert(0.0);
        *a = (*a + d * 0.6).clamp(-2.0, 2.0);
    }
    /// Called whenever a track stops being the current one.
    pub fn on_finished(&mut self, t: &Track, listened_frac: f32, skipped: bool) {
        self.plays[t.id] += 1;
        self.listened_secs += listened_frac * t.dur;
        if skipped && listened_frac < 0.3 {
            self.skips[t.id] += 1;
            self.bump(t, -0.25);
        } else if listened_frac > 0.7 {
            self.bump(t, 0.15);
        } else {
            self.bump(t, 0.03);
        }
        self.history.push(t.id);
    }
    pub fn like(&mut self, t: &Track) {
        self.liked[t.id] = !self.liked[t.id];
        self.disliked[t.id] = false;
        if self.liked[t.id] {
            self.bump(t, 0.6);
        }
    }
    pub fn dislike(&mut self, t: &Track) {
        self.disliked[t.id] = !self.disliked[t.id];
        self.liked[t.id] = false;
        if self.disliked[t.id] {
            self.bump(t, -1.0);
        }
    }
    pub fn replay(&mut self, t: &Track) {
        if self.plays[t.id] > 0 {
            self.bump(t, 0.3);
        }
    }
    pub fn taste(&self, t: &Track) -> f32 {
        let g = self.genre_aff.get(&t.genre).copied().unwrap_or(0.0);
        let a = self.artist_aff.get(&t.artist).copied().unwrap_or(0.0);
        (g + a + self.track_aff[t.id]).tanh()
    }
    pub fn save(&self, lib: &[Track]) {
        let mut s = String::new();
        for (g, v) in &self.genre_aff {
            s.push_str(&format!("g\t{}\t{}\n", g, v));
        }
        for (g, v) in &self.artist_aff {
            s.push_str(&format!("a\t{}\t{}\n", g, v));
        }
        for t in lib {
            s.push_str(&format!(
                "t\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                t.title, t.artist, self.plays[t.id], self.skips[t.id], self.liked[t.id] as u8, self.disliked[t.id] as u8, self.track_aff[t.id]
            ));
        }
        let _ = std::fs::write("dj_profile.txt", s);
    }
    pub fn load(&mut self, lib: &[Track]) {
        let s = match std::fs::read_to_string("dj_profile.txt") {
            Ok(s) => s,
            Err(_) => return,
        };
        self.sessions += 1;
        for line in s.lines() {
            let p: Vec<&str> = line.split('\t').collect();
            match p.first().copied() {
                Some("g") if p.len() == 3 => {
                    self.genre_aff.insert(p[1].to_string(), p[2].parse().unwrap_or(0.0));
                }
                Some("a") if p.len() == 3 => {
                    self.artist_aff.insert(p[1].to_string(), p[2].parse().unwrap_or(0.0));
                }
                Some("t") if p.len() == 8 => {
                    if let Some(t) = lib.iter().find(|t| t.title == p[1] && t.artist == p[2]) {
                        self.plays[t.id] = p[3].parse().unwrap_or(0);
                        self.skips[t.id] = p[4].parse().unwrap_or(0);
                        self.liked[t.id] = p[5] == "1";
                        self.disliked[t.id] = p[6] == "1";
                        self.track_aff[t.id] = p[7].parse().unwrap_or(0.0);
                    }
                }
                _ => {}
            }
        }
    }
}

pub fn key_score(a: &Track, b: &Track) -> f32 {
    key_score_shifted(a, 0, b, 0)
}

/// Camelot code after pitch-shifting by `semis` semitones (one semitone = 7 steps round the wheel).
pub fn shift_cam(c: (i32, char), semis: i32) -> (i32, char) {
    ((c.0 - 1 + 7 * semis).rem_euclid(12) + 1, c.1)
}

/// Key compatibility of B after A, each possibly key-shifted (in semitones).
pub fn key_score_shifted(a: &Track, shift_a: i32, b: &Track, shift_b: i32) -> f32 {
    let (na, la) = shift_cam(camelot(a.a.key, a.a.minor), shift_a);
    let (nb, lb) = shift_cam(camelot(b.a.key, b.a.minor), shift_b);
    let d = {
        let x = (na - nb).abs();
        x.min(12 - x)
    };
    let low_conf = a.a.key_conf < 0.02 || b.a.key_conf < 0.02;
    let s = if d == 0 && la == lb {
        1.0
    } else if d == 0 {
        0.85
    } else if d == 1 && la == lb {
        0.9
    } else if d == 1 {
        0.45
    } else if d == 2 && la == lb {
        0.55
    } else {
        (0.35 - 0.06 * d as f32).max(0.05)
    };
    if low_conf {
        0.5 * s + 0.25
    } else {
        s
    }
}

/// Biggest key shift of the next song (voices sound unnatural beyond this).
pub const MAX_KEY_SHIFT: i32 = 1;
/// Each semitone of shift costs this much score, so a song is shifted only when it clearly helps.
const KEY_SHIFT_PENALTY: f32 = 0.05;
const KEY_SHIFT_MIN_GAIN: f32 = 0.3;
/// Both key detections must be at least this sure before a song is shifted (a wrong key would detune it).
const KEY_SHIFT_MIN_CONF: f32 = 0.12;

/// Key shift (semitones, -1..1) for `b` so it follows `a` (already shifted by `shift_a`) better,
/// and the key score after the shift. 0 unless it clearly helps. Ported from automix.
pub fn best_key_shift(a: &Track, shift_a: i32, b: &Track) -> (i32, f32) {
    let plain = key_score_shifted(a, shift_a, b, 0);
    if a.a.key_conf < KEY_SHIFT_MIN_CONF || b.a.key_conf < KEY_SHIFT_MIN_CONF {
        return (0, plain);
    }
    let (mut best, mut best_value, mut best_score) = (0, plain, plain);
    for sh in [1, -1] {
        let sc = key_score_shifted(a, shift_a, b, sh);
        let v = sc - KEY_SHIFT_PENALTY * sh.abs() as f32;
        if v > best_value + 1e-6 {
            best = sh;
            best_value = v;
            best_score = sc;
        }
    }
    if best != 0 && best_value < plain + KEY_SHIFT_MIN_GAIN {
        return (0, plain);
    }
    (best, best_score)
}

/// (rate multiplier needed on `b`, beat multiplier used, score 0..1)
pub fn tempo_match(out_eff: f32, b: &Track) -> (f64, f32, f32) {
    let r0 = out_eff as f64 / b.a.bpm as f64;
    let mut best = (r0, 1.0f32);
    for (m, mult) in [(0.5f64, 0.5f32), (2.0, 2.0)] {
        let r = r0 * m;
        if r.ln().abs() < best.0.ln().abs() {
            best = (r, mult);
        }
    }
    let d = best.0.ln().abs() as f32;
    // Up to about 16% apart can be beat-matched: the time-stretcher keeps pitch, and
    // with AI layers the drums hand over quickly so stretch artefacts stay short.
    (best.0, best.1, (1.0 - d / 0.16).clamp(0.0, 1.0))
}

pub struct Pick {
    pub score: f32,
    pub reason: String,
}

pub fn score_candidate(
    cand: &Track,
    cur: Option<&Track>,
    cur_eff_bpm: Option<f32>,
    set: &Settings,
    prof: &Profile,
    recent: &[usize],
) -> Pick {
    let mut reasons: Vec<String> = Vec::new();
    let mut score = 0f32;
    let (w_tempo, w_key, w_dance, w_energy) = set.profile.weights();
    let target = set.target_energy();
    let mut tgt_now = target;
    if let Some(c) = cur {
        let step = set.mood.max_step();
        tgt_now = c.energy + (target - c.energy).clamp(-step, step);
        let out_eff = cur_eff_bpm.unwrap_or(c.a.bpm);
        let (r, mult, ts) = tempo_match(out_eff, cand);
        // weights fitted on the training DJ sets: tempo matters most on profile 1
        score += w_tempo * ts;
        if ts > 0.0 {
            let pct = (r - 1.0) * 100.0;
            reasons.push(format!(
                "tempo {:.0} -> {:.0} BPM ({:+.1}% shift{})",
                out_eff,
                cand.a.bpm,
                pct,
                if mult != 1.0 { ", half/double-time" } else { "" }
            ));
        } else {
            reasons.push(format!("tempo gap {:.0} -> {:.0} BPM, will use an echo transition", out_eff, cand.a.bpm));
        }
        let ks = key_score(c, cand);
        score += w_key * ks;
        if ks >= 0.85 {
            reasons.push(format!("harmonic match {} -> {}", camelot_str(c.a.key, c.a.minor), camelot_str(cand.a.key, cand.a.minor)));
        } else if ks >= 0.5 {
            reasons.push(format!("keys {} -> {} are neighbours", camelot_str(c.a.key, c.a.minor), camelot_str(cand.a.key, cand.a.minor)));
        } else {
            reasons.push(format!("key change {} -> {}", camelot_str(c.a.key, c.a.minor), camelot_str(cand.a.key, cand.a.minor)));
        }
        score += w_dance * (1.0 - (cand.dance - c.dance).abs());
        let (wl, wm, wh) = set.profile.band_weights();
        if wl + wm + wh > 0.0 {
            let (sa, sb) = (band_shares(c), band_shares(cand));
            score += wl * (1.0 - (sa[0] - sb[0]).abs() * 2.5).max(0.0)
                + wm * (1.0 - (sa[1] - sb[1]).abs() * 2.5).max(0.0)
                + wh * (1.0 - (sa[2] - sb[2]).abs() * 5.0).max(0.0);
        }
    }
    let es = (1.0 - (cand.energy - tgt_now).abs() * 2.5).clamp(0.0, 1.0);
    score += w_energy * es;
    if let Some(c) = cur {
        let dir = if cand.energy > c.energy + 0.04 {
            "building"
        } else if cand.energy < c.energy - 0.04 {
            "easing"
        } else {
            "holding"
        };
        reasons.push(format!("energy {:.2} -> {:.2} {} toward {} ({:.2})", c.energy, cand.energy, dir, set.mood.name(), target));
    } else {
        reasons.push(format!("energy {:.2} suits {}", cand.energy, set.mood.name()));
    }
    let mt = set.mood.tag_match(cand);
    score += 0.8 * mt;
    let taste = prof.taste(cand);
    score += 1.2 * (0.5 + 0.5 * taste);
    if taste > 0.3 {
        reasons.push(format!("matches your taste in {}", cand.genre));
    }
    let unfamiliar = prof.plays[cand.id] == 0;
    let dw = if set.mood == Mood::Discovery { (set.discovery + 0.4).min(1.0) } else { set.discovery };
    if unfamiliar {
        let near = (prof.genre_aff.get(&cand.genre).copied().unwrap_or(0.0) + 0.5).clamp(0.0, 1.0);
        score += 1.0 * dw * (0.4 + 0.6 * near);
        if dw > 0.3 {
            reasons.push("fresh discovery close to your taste".to_string());
        }
    } else {
        score += 0.8 * (1.0 - dw);
    }
    if let Some(g) = &set.genre {
        if &cand.genre == g {
            score += 0.7;
            reasons.push(format!("preferred genre {}", g));
        }
    }
    if prof.liked[cand.id] {
        score += 0.5;
        reasons.push("a track you liked".to_string());
    }
    if prof.disliked[cand.id] {
        score -= 6.0;
    }
    let skips = prof.skips[cand.id] as f32;
    score -= 0.4 * skips.min(3.0);
    for (j, &r) in recent.iter().rev().enumerate().take(8) {
        if r == cand.id {
            score -= 4.0 - 0.3 * j as f32;
        }
    }
    let mut same_artist = false;
    for &r in recent.iter().rev().take(2) {
        if let Some(c) = cur {
            let _ = c;
        }
        let _ = r;
    }
    if let Some(c) = cur {
        if c.artist == cand.artist {
            same_artist = true;
        }
    }
    if same_artist {
        score -= 1.1;
    } else {
        reasons.push("different artist keeps variety".to_string());
    }
    Pick { score, reason: reasons.join(" | ") }
}

pub struct QItem {
    pub track: usize,
    pub pinned: bool,
    pub manual: bool,
    pub reason: String,
}

/// Picks the best next track given the end of the current chain.
pub fn pick_next(lib: &[Track], chain_end: Option<usize>, eff_bpm: Option<f32>, set: &Settings, prof: &Profile, exclude: &[usize], recent: &[usize]) -> Option<(usize, String)> {
    let cur = chain_end.map(|i| &lib[i]);
    let mut best: Option<(f32, usize, String)> = None;
    for t in lib {
        if exclude.contains(&t.id) {
            continue;
        }
        let p = score_candidate(t, cur, eff_bpm, set, prof, recent);
        if best.as_ref().map(|b| p.score > b.0).unwrap_or(true) {
            best = Some((p.score, t.id, p.reason));
        }
    }
    best.map(|b| (b.1, b.2))
}

/// Greedy re-ordering of unpinned queue entries by musical compatibility.
pub fn smart_sort(lib: &[Track], start: Option<usize>, q: &mut Vec<QItem>, set: &Settings, prof: &Profile) {
    let mut result: Vec<QItem> = Vec::new();
    let mut pool: Vec<QItem> = Vec::new();
    let mut slots: Vec<bool> = Vec::new();
    let old: Vec<QItem> = std::mem::take(q);
    for it in old {
        slots.push(it.pinned);
        if it.pinned {
            result.push(it);
        } else {
            pool.push(it);
        }
    }
    let mut out: Vec<QItem> = Vec::new();
    let mut pinned_iter = result.into_iter();
    let mut chain = start;
    let mut recent: Vec<usize> = prof.history.iter().rev().take(4).cloned().collect();
    recent.reverse();
    for pinned in slots {
        if pinned {
            if let Some(p) = pinned_iter.next() {
                chain = Some(p.track);
                out.push(p);
            }
        } else if !pool.is_empty() {
            let cur = chain.map(|i| &lib[i]);
            let mut bi = 0;
            let mut bs = f32::MIN;
            for (i, it) in pool.iter().enumerate() {
                let p = score_candidate(&lib[it.track], cur, None, set, prof, &recent);
                if p.score > bs {
                    bs = p.score;
                    bi = i;
                }
            }
            let mut it = pool.remove(bi);
            it.reason = score_candidate(&lib[it.track], cur, None, set, prof, &recent).reason;
            chain = Some(it.track);
            out.push(it);
        }
    }
    *q = out;
}

// ------------------------------------------------------------ AutoMix points

#[derive(Clone)]
pub struct MixPoint {
    pub pos: f64,
    pub score: f32,
    pub kind: &'static str,
    pub lvl: f32,
}

pub struct MixChoice {
    pub out_pos: f64,
    pub in_pos: f64,
    pub beats: u32,
    pub why: String,
}

fn bar_len(t: &Track) -> f64 {
    4.0 * crate::tracks::SR as f64 * 60.0 / t.a.bpm as f64
}

fn mean_rms(bars: &[[f32; 3]], a: isize, b: isize) -> f32 {
    let lo = a.max(0) as usize;
    let hi = (b.max(0) as usize).min(bars.len());
    if hi <= lo {
        return 0.0;
    }
    bars[lo..hi].iter().map(|x| x[0]).sum::<f32>() / (hi - lo) as f32
}

fn mean_vocal(bars: &[[f32; 3]], a: isize, b: isize) -> f32 {
    let lo = a.max(0) as usize;
    let hi = (b.max(0) as usize).min(bars.len());
    if hi <= lo {
        return 0.0;
    }
    bars[lo..hi].iter().map(|x| x[2] / (x[0] + 1e-6)).sum::<f32>() / (hi - lo) as f32
}

fn track_levels(t: &Track) -> (f32, f32) {
    let bars = &t.a.bars;
    let (fb, lb) = (t.a.first_bar, t.a.last_bar.min(bars.len()));
    if lb <= fb {
        return (1e-6, 1.0);
    }
    let mut r: Vec<f32> = bars[fb..lb].iter().map(|x| x[0]).collect();
    r.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let level = r[((r.len() as f32 * 0.8) as usize).min(r.len() - 1)].max(1e-6);
    let mut v: Vec<f32> = bars[fb..lb].iter().map(|x| x[2] / (x[0] + 1e-6)).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (level, v[v.len() / 2].max(1e-3))
}

/// Places in the song where leaving is musically good: before an energy drop
/// (breakdown / outro), at a phrase end, and away from vocals. Anywhere in the song.
pub fn out_points(t: &Track) -> Vec<MixPoint> {
    let bars = &t.a.bars;
    let (fb, lb) = (t.a.first_bar, t.a.last_bar.min(bars.len()));
    let mut v = Vec::new();
    if lb < fb + 8 {
        return v;
    }
    let (level, med) = track_levels(t);
    let ba = bar_len(t);
    let mut i = fb + 4;
    while i <= lb {
        let ep = mean_rms(bars, i as isize - 4, i as isize);
        let en = if i >= lb { 0.0 } else { mean_rms(bars, i as isize, i as isize + 4) };
        let drop = ((ep - en) / ep.max(1e-6)).clamp(0.0, 1.0);
        let m = if i >= lb { mean_vocal(bars, i as isize - 4, i as isize) } else { mean_vocal(bars, i as isize, i as isize + 4) };
        let vf = (1.2 - m / med).clamp(0.0, 1.0);
        let end = i + 4 >= lb;
        let kind = if end {
            "outro"
        } else if drop > 0.25 {
            "energy drop"
        } else {
            "phrase end"
        };
        // pro DJs mix on phrase lines: 8 bars good, 16/32-bar phrase starts better
        let phrase = if (i - fb) % 16 == 0 { 0.35 } else if (i - fb) % 8 == 0 { 0.2 } else { 0.0 };
        let score = 0.9 * drop + 0.4 * vf + phrase + if end { 0.6 } else { 0.0 };
        v.push(MixPoint { pos: t.a.origin + i as f64 * ba, score, kind, lvl: ep / level });
        i += 4;
    }
    v
}

/// Places where a song can be entered: its opening, or a later section that builds up.
pub fn in_points(t: &Track) -> Vec<MixPoint> {
    let bars = &t.a.bars;
    let (fb, lb) = (t.a.first_bar, t.a.last_bar.min(bars.len()));
    let mut v = Vec::new();
    if lb < fb + 8 {
        return v;
    }
    let (level, med) = track_levels(t);
    let ba = bar_len(t);
    let mut j = fb;
    while j + 8 <= lb {
        let en = mean_rms(bars, j as isize, j as isize + 4);
        let ep = mean_rms(bars, j as isize - 4, j as isize);
        let rise = if j == fb { 0.0 } else { ((en - ep) / en.max(1e-6)).clamp(0.0, 1.0) };
        let vf = (1.2 - mean_vocal(bars, j as isize, j as isize + 4) / med).clamp(0.0, 1.0);
        let start = if j == fb { 0.7 } else { 0.0 };
        let kind = if j == fb {
            "opening"
        } else if rise > 0.25 {
            "build-up"
        } else {
            "section start"
        };
        v.push(MixPoint { pos: t.a.origin + j as f64 * ba, score: 0.5 * rise + start + 0.4 * vf + if (j - fb) % 16 == 0 { 0.35 } else if (j - fb) % 8 == 0 { 0.2 } else { 0.0 }, kind, lvl: en / level });
        j += 4;
    }
    v
}

fn clock(samples: f64) -> String {
    let s = (samples / crate::tracks::SR as f64).max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// (low, mid, high) energy shares of a track, from its analysis.
pub fn band_shares(t: &Track) -> [f32; 3] {
    let hi = t.a.bright * t.a.bright;
    let mid = t.a.vocal * t.a.vocal;
    [(1.0 - hi - mid).max(0.0), mid, hi]
}

/// Sample position where a track's melody (mid band) comes in, as measured by the trainer:
/// the first bar whose mid band reaches 55% of its typical level.
pub fn melody_onset(t: &Track) -> f64 {
    let end = t.a.last_bar.min(t.a.bars.len());
    let mut v: Vec<f32> = (t.a.first_bar..end).map(|b| t.a.bars[b][2]).collect();
    if v.is_empty() {
        return t.a.origin;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let typ = v[v.len() * 6 / 10];
    let bar = bar_len(t);
    for b in t.a.first_bar..end {
        if t.a.bars[b][2] >= 0.55 * typ {
            return t.a.origin + b as f64 * bar;
        }
    }
    t.a.origin
}

/// Person's choices that override the automatic search.
#[derive(Clone, Copy, Default)]
pub struct MixOpts {
    pub force_out: Option<f64>,
    pub force_in: Option<f64>,
    /// Latest position (end of a selected section) the outgoing song may play to.
    pub out_limit: Option<f64>,
    /// Quick auto mix: the outgoing song must be left inside this window (start, end) in samples;
    /// every bar line in it is a candidate.
    pub out_window: Option<(f64, f64)>,
    /// Profile 3: the DJs in the third training mix came in after the melody had started
    /// (18 of 20 songs, median ~3 bars after), so entries before it are penalised.
    pub after_melody: bool,
    /// DJ feel: aim to leave the outgoing song around where the training DJs did (about 2.5 min
    /// before its end, ~45 s of play) instead of letting it play out.
    pub club: bool,
}

/// The most useful exit points of a song (any location), in time order.
pub fn top_out_points(t: &Track, after: f64, max: usize) -> Vec<MixPoint> {
    let mut v: Vec<MixPoint> = out_points(t).into_iter().filter(|p| p.pos > after && p.score > 0.3).collect();
    v.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    v.truncate(max);
    v.sort_by(|a, b| a.pos.partial_cmp(&b.pos).unwrap());
    v
}

/// The most useful entry points of a song (any location), in time order.
pub fn top_in_points(t: &Track, max: usize) -> Vec<MixPoint> {
    let mut v: Vec<MixPoint> = in_points(t).into_iter().filter(|p| p.score > 0.3).collect();
    v.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    v.truncate(max);
    v.sort_by(|a, b| a.pos.partial_cmp(&b.pos).unwrap());
    v
}

pub fn clock_str(samples: f64) -> String {
    clock(samples)
}

fn nearest_level(points: &[MixPoint], pos: f64) -> f32 {
    points.iter().min_by(|a, b| (a.pos - pos).abs().partial_cmp(&(b.pos - pos).abs()).unwrap()).map(|p| p.lvl).unwrap_or(0.7)
}

/// Blend lengths (beats) tried in addition to the caller's own `beats`, when they fit
/// under its cap. `choose_mix` scores every one of them, so a length that gives a worse
/// (exit, entry) pair loses to a length that gives a better one, instead of the length
/// being fixed before any point is looked at.
const BEAT_OPTIONS: [u32; 4] = [8, 16, 24, 32];
/// How much a transition's own length (long when clean, short when busy) can move its
/// score, relative to the point-placement terms above (roughly LENGTH_WEIGHT in automix).
const LENGTH_WEIGHT: f32 = 0.3;
/// Penalty for both songs carrying lead vocals through the overlap.
const VOCAL_CLASH_WEIGHT: f32 = 0.9;

/// AutoMix: looks through the whole of both songs for the best (exit point, entry
/// point, length) for a transition at or after `pos`. `beats_cap` is the longest blend
/// the caller allows (the style/intensity rules pick this); shorter lengths are tried
/// too and may win when they fit the music better. A section chosen by the person
/// limits where the outgoing song may be left and where the next one starts.
pub fn choose_mix(ta: &Track, tin: &Track, pos: f64, rate: f64, beats_cap: u32, calm: bool, opts: MixOpts, vp: Option<(&crate::vibe::Profile, &crate::vibe::Profile)>) -> Option<MixChoice> {
    let mut lengths: Vec<u32> = BEAT_OPTIONS.iter().copied().filter(|&b| b <= beats_cap).collect();
    if !lengths.contains(&beats_cap) {
        lengths.push(beats_cap);
    }
    if lengths.is_empty() {
        lengths.push(beats_cap.max(1));
    }
    let max_bars = (lengths.iter().copied().max().unwrap_or(1) as f32 / 4.0).max(1.0);

    let sr = crate::tracks::SR as f64;
    let ba = bar_len(ta);
    let bb = bar_len(tin);
    let earliest = pos + 3.0 * sr * rate;
    let len_a = ta.samples() as f64;
    let len_b = tin.samples() as f64;
    let a_end = (ta.a.origin + ta.a.last_bar as f64 * ba).min(len_a);
    let b_end = (tin.a.origin + tin.a.last_bar as f64 * bb).min(len_b);
    let min_pos = (40.0f64).min(0.5 * ta.dur as f64) * sr;
    let mut outs = out_points(ta);
    let mut ins = in_points(tin);
    if let Some(f) = opts.force_out {
        let lvl = nearest_level(&outs, f);
        outs = vec![MixPoint { pos: f, score: 1.0, kind: "your choice", lvl }];
    }
    if let Some(f) = opts.force_in {
        let lvl = nearest_level(&ins, f);
        ins = vec![MixPoint { pos: f, score: 1.0, kind: "your choice", lvl }];
    }
    let win = opts.out_window.filter(|_| opts.force_out.is_none());
    if let Some((lo, hi)) = win {
        let first = ((lo - ta.a.origin) / ba).ceil().max(0.0) as i64;
        let last = ((hi - ta.a.origin) / ba).floor() as i64;
        let mut v = Vec::new();
        for i in first..=last.min(first + 16) {
            let pos = ta.a.origin + i as f64 * ba;
            let lvl = nearest_level(&outs, pos);
            v.push(MixPoint { pos, score: 0.5, kind: "bar line", lvl });
        }
        outs = v;
    }
    let melody_in = melody_onset(tin);
    let (_, med_a) = track_levels(ta);
    let (_, med_b) = track_levels(tin);
    let limited = opts.out_limit.is_some() || opts.force_out.is_some() || win.is_some();
    let mut best: Option<(f32, MixPoint, MixPoint, u32, String)> = None;
    for &beats in &lengths {
        let trans_bars = (beats as f64 / 4.0).ceil().max(1.0);
        for o in &outs {
            if win.is_none() && o.pos < earliest && opts.force_out.is_none() {
                continue;
            }
            if win.is_none() && opts.force_out.is_none() && o.pos + trans_bars * ba > a_end + ba {
                continue;
            }
            if let Some(lim) = opts.out_limit {
                if o.pos + trans_bars * ba > lim + 0.5 * ba {
                    continue;
                }
            }
            let ob = vp.map(|(pa, _)| crate::vibe::out_bonus(pa, o.pos, trans_bars));
            let mut tpen = 0.0f32;
            if !limited {
                tpen = 0.9 * (1.0 - (o.pos / (0.7 * len_a)).min(1.0)) as f32;
                if opts.club {
                    // training DJs left songs with ~156 s unplayed (median), never before ~50 s in
                    let target = (len_a - 156.0 * sr).max(50.0 * sr).min(0.6 * len_a);
                    tpen = 1.4 * ((o.pos - target).abs() / (45.0 * sr)).min(1.5) as f32;
                }
                if o.pos < min_pos {
                    tpen += 1.0;
                }
                // The layer bonuses can add up to ~3 points; without this an attractive early
                // spot could cut a song short. Play at least ~55% of the song on its own.
                // Club pacing (training sets: songs play ~40 s, 15-27% of their length) when the
                // transition is short and punchy; long calm blends keep more of the song.
                let keep = if opts.club { 0.15 } else if calm || beats >= 32 { 0.55 } else { 0.3 };
                if vp.is_some() && o.pos < keep * a_end {
                    continue;
                }
            } else if let Some(lim) = opts.out_limit {
                // inside a chosen section prefer leaving as late as the section allows
                tpen = 0.8 * ((lim - o.pos) / (lim.max(1.0))).clamp(0.0, 1.0) as f32;
            }
            for n in &ins {
                if opts.force_in.is_none() && n.pos + (trans_bars + 4.0) * bb > b_end {
                    continue;
                }
                let lvl_cont = 1.0 - (o.lvl - n.lvl).abs().min(1.0);
                // Two lead vocals at once is the classic clash pro DJs avoid: EQ cannot make two
                // lyrics readable, so overlaps where both songs sing are marked down.
                let vclash = if opts.force_in.is_none() && opts.force_out.is_none() {
                    let ob0 = ((o.pos - ta.a.origin) / ba) as isize;
                    let nb0 = ((n.pos - tin.a.origin) / bb) as isize;
                    let tb = trans_bars as isize;
                    let va = (mean_vocal(&ta.a.bars, ob0, ob0 + tb) / med_a).clamp(0.0, 2.0);
                    let vb = (mean_vocal(&tin.a.bars, nb0, nb0 + tb) / med_b).clamp(0.0, 2.0);
                    ((va - 0.8).max(0.0) * (vb - 0.8).max(0.0)).min(1.0)
                } else {
                    0.0
                };
                let (cont, vibe_why) = match vp {
                    Some((pa, pb)) => {
                        let (ib, iw) = crate::vibe::in_bonus(pb, n.pos, trans_bars);
                        let (ow, oww) = ob.unwrap();
                        let (c, cw) = crate::vibe::continuity(pa, o.pos, pb, n.pos, trans_bars);
                        (0.3 * lvl_cont + 1.4 * c + ow + ib, format!("; {}, {}; {}", oww, iw, cw))
                    }
                    None => (0.8 * lvl_cont, String::new()),
                };
                let mut s = -VOCAL_CLASH_WEIGHT * vclash + o.score * if calm && o.kind == "outro" { 1.3 } else { 1.0 } + n.score * if calm && n.kind == "opening" { 1.4 } else { 1.0 } + cont - tpen;
                if !calm && n.kind != "opening" {
                    s += 0.15;
                }
                if opts.after_melody && opts.force_in.is_none() && n.pos + 1.0 * bb < melody_in {
                    s -= 0.8;
                }
                if opts.force_in.is_none() {
                    let skipped = ((n.pos - tin.a.origin) / len_b.max(1.0)) as f32;
                    s -= 0.6 * skipped * if calm { 1.5 } else { 1.0 };
                }
                // Both tracks loud through the overlap (crowded) favours a short
                // transition; both quiet (clean) can afford a long one.
                let crowded = (0.5 * (o.lvl + n.lvl)).clamp(0.0, 1.0);
                s += LENGTH_WEIGHT * (trans_bars as f32 / max_bars) * (1.0 - 2.0 * crowded);
                if best.as_ref().map(|b| s > b.0).unwrap_or(true) {
                    best = Some((s, o.clone(), n.clone(), beats, vibe_why));
                }
            }
        }
    }
    best.map(|(_, o, n, beats, vw)| MixChoice {
        out_pos: o.pos,
        in_pos: n.pos,
        beats,
        why: format!("leaves {} at {} ({}), enters {} at {} ({}){}", ta.title, clock(o.pos), o.kind, tin.title, clock(n.pos), n.kind, vw),
    })
}

// ------------------------------------------------- AutoMix energy + alignment helpers

fn body_level(t: &Track) -> f32 {
    let mut v: Vec<f32> = t.a.bars.iter().map(|b| b[0]).filter(|&x| x > 1e-4).collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[(v.len() as f32 * 0.6) as usize % v.len()]
}

/// Sample position where the song's loud content ends (before any fade-out or quiet tail).
/// A transition should be finished by here, or the music sags into dead air.
pub fn loud_end(t: &Track) -> f64 {
    let body = body_level(t);
    let bar = bar_len(t);
    let mut k = t.a.bars.len();
    while k > 0 && t.a.bars[k - 1][0] < 0.5 * body {
        k -= 1;
    }
    (t.a.origin + k as f64 * bar).min(t.samples() as f64)
}

/// Seconds from `from` until the song reaches its normal loudness (length of a quiet intro).
pub fn quiet_intro_secs(t: &Track, from: f64) -> f32 {
    let body = body_level(t);
    let bar = bar_len(t);
    for (i, b) in t.a.bars.iter().enumerate() {
        let p = t.a.origin + i as f64 * bar;
        if p + bar <= from {
            continue;
        }
        if b[0] >= 0.5 * body {
            return ((p - from).max(0.0) / crate::tracks::SR as f64) as f32;
        }
    }
    0.0
}

/// Blend length (beats) that fits the next song: it should reach full power about when the
/// blend ends. `intro_secs` is the quiet intro (see `quiet_intro_secs`), `bpm` the tempo both
/// songs play at, `cap` the longest blend the intensity setting allows. A song that starts
/// loud gets a short blend (8 beats); calm moods never go below 16. Ported from automix.
pub fn fit_blend_beats(intro_secs: f32, bpm: f32, cap: u32, calm: bool) -> u32 {
    let want = if intro_secs < 2.0 {
        8
    } else {
        let beats = intro_secs * bpm.max(40.0) / 60.0;
        (((beats / 4.0).round() as u32) * 4).clamp(8, 32)
    };
    let n = want.min(cap);
    if calm {
        n.max(16)
    } else {
        n
    }
}

/// Onset envelope of `pcm` read from `start` at playback `rate`, on a 5 ms output-time grid.
/// Low-pass filtered to the kick/bass band (~150 Hz) first: on vocal or acoustic songs the
/// full-band energy is dominated by strums and singers, which pulls the cross-correlation
/// in `align_entry` onto the wrong hit; the beat itself lives in the low end.
fn onset_track(pcm: &[f32], start: f64, rate: f64, steps: usize) -> Vec<f32> {
    let sr = crate::tracks::SR as f32;
    let a_lp = 1.0 - (-2.0 * std::f32::consts::PI * 150.0 / sr).exp();
    let h = crate::tracks::SR as f64 / 200.0;
    let mut e = Vec::with_capacity(steps);
    for k in 0..steps {
        let a = (start + k as f64 * h * rate) as usize;
        let b = ((start + (k + 1) as f64 * h * rate) as usize).min(pcm.len());
        if a >= b {
            e.push(0.0);
            continue;
        }
        let mut lp = 0f32;
        let mut s = 0f32;
        for &x in &pcm[a..b] {
            lp += a_lp * (x - lp);
            s += lp * lp;
        }
        e.push((s / (b - a) as f32).sqrt());
    }
    (0..e.len()).map(|i| if i == 0 { 0.0 } else { (e[i] - e[i - 1]).max(0.0) }).collect()
}

/// Nudges the incoming entry point so its drum hits land on the outgoing song's hits
/// over the first seconds of the overlap (cross-correlation within half a beat).
pub fn align_entry(a: &[f32], a_pos: f64, a_rate: f64, b: &[f32], b_pos: f64, b_rate: f64, beat_secs: f64, window_secs: f64) -> f64 {
    let steps = (window_secs * 200.0) as usize;
    let max_lag = ((beat_secs * 0.5) * 200.0) as i64;
    let oa = onset_track(a, a_pos, a_rate, steps);
    let ob = onset_track(b, (b_pos - max_lag as f64 * crate::tracks::SR as f64 / 200.0 * b_rate).max(0.0), b_rate, steps + 2 * max_lag as usize);
    // ob[k + max_lag] is aligned with oa[k] at lag 0
    let mut best = (f32::MIN, 0i64);
    for lag in -max_lag..=max_lag {
        let mut s = 0f32;
        for k in 0..steps {
            let j = k as i64 + max_lag + lag;
            if j >= 0 && (j as usize) < ob.len() {
                s += oa[k] * ob[j as usize];
            }
        }
        if s > best.0 {
            best = (s, lag);
        }
    }
    // a hit of B that shows up `lag` steps late means B must start that much further in
    (b_pos + best.1 as f64 * crate::tracks::SR as f64 / 200.0 * b_rate).max(0.0)
}

#[cfg(test)]
mod key_shift_tests {
    use super::*;

    #[test]
    fn camelot_shift_matches_the_wheel() {
        // one semitone up = 7 steps round the wheel, letter unchanged
        assert_eq!(shift_cam((8, 'B'), 1), (3, 'B'));
        assert_eq!(shift_cam((8, 'A'), -1), (1, 'A'));
        assert_eq!(shift_cam((5, 'A'), 2), (7, 'A'));
        assert_eq!(shift_cam((5, 'A'), 0), (5, 'A'));
    }
}

/// A click track: short decaying bursts every `beat_secs`, starting at `phase_secs`.
/// Used by `--align-test` to probe `align_entry` with a known, exact phase offset.
fn click_track(len_secs: f64, beat_secs: f64, phase_secs: f64) -> Vec<f32> {
    let sr = crate::tracks::SR as f64;
    let n = (len_secs * sr) as usize;
    let mut v = vec![0f32; n];
    let beat = beat_secs * sr;
    let mut t = phase_secs * sr;
    while (t as usize) < n {
        for k in 0..200usize {
            let idx = t as usize + k;
            if idx >= n {
                break;
            }
            v[idx] += (1.0 - k as f32 / 200.0) * 0.8;
        }
        t += beat;
    }
    v
}

/// Synthetic check: builds two click tracks with a known phase offset and prints what
/// `align_entry` recovers, to tell apart an alignment-algorithm bug from a pipeline bug.
pub fn align_self_test() {
    let sr = crate::tracks::SR as f64;
    let beat = 0.5; // 120 BPM
    let a = click_track(8.0, beat, 0.0);
    for &true_shift in &[0.02, 0.05, 0.08, 0.12, 0.18, -0.08, -0.18] {
        let b = click_track(8.0, beat, true_shift);
        let a_pos = 4.0 * sr;
        let b_pos = 4.0 * sr;
        let new_b = align_entry(&a, a_pos, 1.0, &b, b_pos, 1.0, beat, 3.0);
        let got = (new_b - b_pos) / sr;
        println!("true shift {:+.3}s -> align_entry recovered {:+.3}s (error {:+.3}s)", true_shift, got, got - true_shift);
    }
}

#[cfg(test)]
mod fit_tests {
    use super::*;

    #[test]
    fn blend_length_follows_the_intro_of_the_next_song() {
        // starts loud: short
        assert_eq!(fit_blend_beats(0.0, 120.0, 16, false), 8);
        // 6 s intro at 120 BPM = 12 beats
        assert_eq!(fit_blend_beats(6.0, 120.0, 32, false), 12);
        // 14 s intro = 28 beats, but the intensity setting only allows 16
        assert_eq!(fit_blend_beats(14.0, 120.0, 16, false), 16);
        assert_eq!(fit_blend_beats(14.0, 120.0, 32, false), 28);
        // calm moods never go below 16
        assert_eq!(fit_blend_beats(0.0, 120.0, 16, true), 16);
        // high intensity (cap 8) stays punchy
        assert_eq!(fit_blend_beats(14.0, 120.0, 8, false), 8);
    }
}
