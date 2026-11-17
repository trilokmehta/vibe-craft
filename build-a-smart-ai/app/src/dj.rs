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

#[derive(Clone)]
pub struct Settings {
    pub auto: bool,
    pub mood: Mood,
    pub energy: f32,
    pub genre: Option<String>,
    pub intensity: f32,
    pub discovery: f32,
}

impl Settings {
    pub fn new() -> Settings {
        Settings { auto: true, mood: Mood::Party, energy: 0.5, genre: None, intensity: 0.5, discovery: 0.35 }
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
pub const MAX_KEY_SHIFT: i32 = 2;
/// Each semitone of shift costs this much score, so a song is shifted only when it clearly helps.
const KEY_SHIFT_PENALTY: f32 = 0.05;
const KEY_SHIFT_MIN_GAIN: f32 = 0.2;
/// Both key detections must be at least this sure before a song is shifted (a wrong key would detune it).
const KEY_SHIFT_MIN_CONF: f32 = 0.05;

/// Key shift (semitones, -2..2) for `b` so it follows `a` (already shifted by `shift_a`) better,
/// and the key score after the shift. 0 unless it clearly helps. Ported from automix.
pub fn best_key_shift(a: &Track, shift_a: i32, b: &Track) -> (i32, f32) {
    let plain = key_score_shifted(a, shift_a, b, 0);
    if a.a.key_conf < KEY_SHIFT_MIN_CONF || b.a.key_conf < KEY_SHIFT_MIN_CONF {
        return (0, plain);
    }
    let (mut best, mut best_value, mut best_score) = (0, plain, plain);
    for sh in [1, -1, 2, -2] {
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
    let target = set.target_energy();
    let mut tgt_now = target;
    if let Some(c) = cur {
        let step = set.mood.max_step();
        tgt_now = c.energy + (target - c.energy).clamp(-step, step);
        let out_eff = cur_eff_bpm.unwrap_or(c.a.bpm);
        let (r, mult, ts) = tempo_match(out_eff, cand);
        // weights fitted on the training DJ sets (55+ real transitions): tempo matters most
        score += 3.0 * ts;
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
        score += 0.9 * ks;
        if ks >= 0.85 {
            reasons.push(format!("harmonic match {} -> {}", camelot_str(c.a.key, c.a.minor), camelot_str(cand.a.key, cand.a.minor)));
        } else if ks >= 0.5 {
            reasons.push(format!("keys {} -> {} are neighbours", camelot_str(c.a.key, c.a.minor), camelot_str(cand.a.key, cand.a.minor)));
        } else {
            reasons.push(format!("key change {} -> {}", camelot_str(c.a.key, c.a.minor), camelot_str(cand.a.key, cand.a.minor)));
        }
        score += 0.7 * (1.0 - (cand.dance - c.dance).abs());
    }
    let es = (1.0 - (cand.energy - tgt_now).abs() * 2.5).clamp(0.0, 1.0);
    score += 1.4 * es;
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
        let phrase = if (i - fb) % 8 == 0 { 0.2 } else { 0.0 };
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
        v.push(MixPoint { pos: t.a.origin + j as f64 * ba, score: 0.5 * rise + start + 0.4 * vf + if (j - fb) % 8 == 0 { 0.2 } else { 0.0 }, kind, lvl: en / level });
        j += 4;
    }
    v
}

fn clock(samples: f64) -> String {
    let s = (samples / crate::tracks::SR as f64).max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
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

/// AutoMix: looks through the whole of both songs for the best pair of
/// (exit point, entry point) at or after `pos`. A section chosen by the
/// person limits where the outgoing song may be left and where the next one starts.
pub fn choose_mix(ta: &Track, tin: &Track, pos: f64, rate: f64, beats: u32, calm: bool, opts: MixOpts, vp: Option<(&crate::vibe::Profile, &crate::vibe::Profile)>) -> Option<MixChoice> {
    let sr = crate::tracks::SR as f64;
    let ba = bar_len(ta);
    let bb = bar_len(tin);
    let trans_bars = (beats as f64 / 4.0).ceil().max(1.0);
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
    let limited = opts.out_limit.is_some() || opts.force_out.is_some() || win.is_some();
    let mut best: Option<(f32, MixPoint, MixPoint, String)> = None;
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
            if o.pos < min_pos {
                tpen += 1.0;
            }
            // The layer bonuses can add up to ~3 points; without this an attractive early
            // spot could cut a song short. Play at least ~55% of the song on its own.
            // Club pacing (training sets: songs play ~40 s, 15-27% of their length) when the
            // transition is short and punchy; long calm blends keep more of the song.
            let keep = if calm || beats >= 32 { 0.55 } else { 0.3 };
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
            let (cont, vibe_why) = match vp {
                Some((pa, pb)) => {
                    let (ib, iw) = crate::vibe::in_bonus(pb, n.pos, trans_bars);
                    let (ow, oww) = ob.unwrap();
                    let (c, cw) = crate::vibe::continuity(pa, o.pos, pb, n.pos, trans_bars);
                    (0.3 * lvl_cont + 1.4 * c + ow + ib, format!("; {}, {}; {}", oww, iw, cw))
                }
                None => (0.8 * lvl_cont, String::new()),
            };
            let mut s = o.score * if calm && o.kind == "outro" { 1.3 } else { 1.0 } + n.score * if calm && n.kind == "opening" { 1.4 } else { 1.0 } + cont - tpen;
            if !calm && n.kind != "opening" {
                s += 0.15;
            }
            if opts.force_in.is_none() {
                let skipped = ((n.pos - tin.a.origin) / len_b.max(1.0)) as f32;
                s -= 0.6 * skipped * if calm { 1.5 } else { 1.0 };
            }
            if best.as_ref().map(|b| s > b.0).unwrap_or(true) {
                best = Some((s, o.clone(), n.clone(), vibe_why));
            }
        }
    }
    best.map(|(_, o, n, vw)| MixChoice {
        out_pos: o.pos,
        in_pos: n.pos,
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

/// Onset envelope of `pcm` read from `start` at playback `rate`, on a 5 ms output-time grid.
fn onset_track(pcm: &[f32], start: f64, rate: f64, steps: usize) -> Vec<f32> {
    let h = crate::tracks::SR as f64 / 200.0;
    let mut e = Vec::with_capacity(steps);
    for k in 0..steps {
        let a = (start + k as f64 * h * rate) as usize;
        let b = ((start + (k + 1) as f64 * h * rate) as usize).min(pcm.len());
        if a >= b {
            e.push(0.0);
            continue;
        }
        let s: f32 = pcm[a..b].iter().map(|x| x * x).sum::<f32>() / (b - a) as f32;
        e.push(s.sqrt());
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
