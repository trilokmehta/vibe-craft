//! Music library manager, procedural track synthesis, and audio analysis
//! (BPM, beat grid, key, energy, sections). Everything runs offline.
use std::f32::consts::PI;
use std::sync::Arc;

pub const SR: u32 = 22050;
pub const HOP: usize = 64;

// ---------------------------------------------------------------- synthesis

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

#[derive(Clone)]
pub struct Spec {
    pub title: &'static str,
    pub artist: &'static str,
    pub album: &'static str,
    pub genre: &'static str,
    pub bpm: f32,
    pub root: u8,
    pub minor: bool,
    pub drive: f32,
    pub style: u8,
    pub seed: u32,
    pub hue: f32,
}

fn sp(
    title: &'static str,
    artist: &'static str,
    album: &'static str,
    genre: &'static str,
    bpm: f32,
    root: u8,
    minor: bool,
    drive: f32,
    style: u8,
    hue: f32,
) -> Spec {
    Spec { title, artist, album, genre, bpm, root, minor, drive, style, seed: (bpm as u32) * 977 + root as u32 * 31 + 7, hue }
}

pub fn specs() -> Vec<Spec> {
    vec![
        sp("Midnight Circuit", "Nova Drift", "Neon Hours", "House", 124.0, 9, true, 0.70, 0, 270.0),
        sp("Glass Elevator", "Nova Drift", "Neon Hours", "House", 122.0, 4, true, 0.65, 0, 245.0),
        sp("Skyline Rush", "Kito Vale", "Rooftop Sessions", "House", 126.0, 2, true, 0.80, 0, 200.0),
        sp("Concrete Pulse", "Mara Voss", "Machine Hearts", "Techno", 132.0, 0, true, 0.90, 0, 350.0),
        sp("Static Bloom", "Mara Voss", "Machine Hearts", "Techno", 130.0, 7, true, 0.85, 0, 330.0),
        sp("Velvet Hour", "Luna Reyes", "Slow Dancing", "Chill", 84.0, 5, false, 0.30, 1, 30.0),
        sp("Paper Lanterns", "Hazel Moon", "Late Tea", "Lo-fi", 78.0, 9, true, 0.25, 1, 50.0),
        sp("Tidewater", "Hazel Moon", "Late Tea", "Lo-fi", 90.0, 3, false, 0.30, 1, 190.0),
        sp("Back Alley Gold", "Rook & Co", "Street Tapes", "Hip-Hop", 92.0, 7, true, 0.55, 3, 40.0),
        sp("Sunset Boulevard", "Aria Lane", "Golden State", "Pop", 108.0, 0, false, 0.65, 2, 20.0),
        sp("Electric Heart", "Aria Lane", "Golden State", "Pop", 112.0, 7, false, 0.72, 2, 320.0),
        sp("Chrome Horizon", "The Afterglow", "Retrowave", "Synthwave", 100.0, 11, true, 0.60, 2, 290.0),
        sp("Overdrive", "Pulse Theory", "Max Effort", "Workout", 128.0, 6, true, 0.95, 0, 5.0),
        sp("Last Dance", "Luna Reyes", "Slow Dancing", "Romantic", 76.0, 8, false, 0.20, 4, 335.0),
        sp("Deep Focus Loop", "Ambient Atlas", "Quiet Rooms", "Focus", 85.0, 2, false, 0.25, 4, 160.0),
    ]
}

fn midi_freq(m: f32) -> f32 {
    440.0 * 2f32.powf((m - 69.0) / 12.0)
}

fn add(buf: &mut [f32], i: usize, v: f32) {
    if i < buf.len() {
        buf[i] += v;
    }
}

fn kick(buf: &mut [f32], at: usize, amp: f32) {
    let n = (SR as f32 * 0.28) as usize;
    let mut ph = 0f32;
    for i in 0..n {
        let t = i as f32 / SR as f32;
        let f = 45.0 + 110.0 * (-t * 28.0).exp();
        ph += 2.0 * PI * f / SR as f32;
        let e = (-t * 9.0).exp();
        let click = if i < 40 { (1.0 - i as f32 / 40.0) * 0.5 } else { 0.0 };
        add(buf, at + i, amp * (ph.sin() * e + click));
    }
}

fn hat(buf: &mut [f32], rng: &mut Rng, at: usize, amp: f32) {
    let n = (SR as f32 * 0.07) as usize;
    let mut prev = 0f32;
    for i in 0..n {
        let t = i as f32 / SR as f32;
        let x = rng.next();
        let y = x - prev;
        prev = x;
        add(buf, at + i, amp * y * (-t * 70.0).exp());
    }
}

fn clap(buf: &mut [f32], rng: &mut Rng, at: usize, amp: f32) {
    let n = (SR as f32 * 0.2) as usize;
    let mut prev = 0f32;
    for i in 0..n {
        let t = i as f32 / SR as f32;
        let x = rng.next();
        let y = x - 0.6 * prev;
        prev = x;
        let burst = if i < 600 { 1.0 + 0.5 * (t * 300.0 * PI).sin() } else { 1.0 };
        add(buf, at + i, amp * 0.5 * y * burst * (-t * 20.0).exp());
    }
}

#[allow(clippy::too_many_arguments)]
fn tone(buf: &mut [f32], at: usize, freq: f32, len: usize, amp: f32, attack: usize, decay: f32, h2: f32, h3: f32) {
    let w = 2.0 * PI * freq / SR as f32;
    let mut ph = 0f32;
    for i in 0..len {
        let t = i as f32 / SR as f32;
        let a = if i < attack { i as f32 / attack.max(1) as f32 } else { 1.0 };
        let rel = ((len - i) as f32 / 200.0).min(1.0);
        let v = ph.sin() + h2 * (2.0 * ph).sin() + h3 * (3.0 * ph).sin();
        ph += w;
        if ph > 2.0 * PI {
            ph -= 2.0 * PI;
        }
        add(buf, at + i, amp * a * rel * (-t * decay).exp() * v);
    }
}

fn pattern(style: u8) -> (Vec<usize>, Vec<usize>, Vec<usize>, Vec<usize>) {
    match style {
        0 => (vec![0, 4, 8, 12], vec![4, 12], vec![2, 6, 10, 14], vec![2, 6, 10, 14]),
        1 => (vec![0, 10], vec![4, 12], vec![0, 2, 4, 6, 8, 10, 12, 14], vec![0, 6, 10]),
        2 => (vec![0, 4, 8, 12], vec![4, 12], vec![0, 2, 4, 6, 8, 10, 12, 14], vec![0, 2, 4, 6, 8, 10, 12, 14]),
        3 => (vec![0, 7, 10], vec![4, 12], vec![0, 2, 4, 6, 8, 10, 12, 14], vec![0, 7, 10]),
        _ => (vec![0, 4, 8, 12], vec![], vec![], vec![0]),
    }
}

fn section(bar: usize) -> u8 {
    match bar {
        0..=7 => 0,
        8..=15 => 1,
        16..=19 => 2,
        20..=27 => 3,
        _ => 4,
    }
}

pub const BARS: usize = 36;

/// Synthesises one track: intro (drums), main, breakdown, main, outro (drums).
pub fn render(s: &Spec) -> Vec<f32> {
    let sr = SR as f32;
    let spb = sr * 60.0 / s.bpm;
    let total = (BARS as f32 * 4.0 * spb) as usize + SR as usize;
    let mut buf = vec![0f32; total];
    let mut rng = Rng(s.seed.wrapping_mul(2654435761) | 1);
    let prog: [i32; 4] = if s.minor { [0, 8, 3, 10] } else { [0, 7, 9, 5] };
    let qual: [i32; 4] = if s.minor { [3, 4, 4, 4] } else { [4, 4, 3, 4] };
    let (kick_steps, clap_steps, hat_steps, bass_steps) = pattern(s.style);
    let bass_len_steps = match s.style {
        0 => 2,
        1 => 6,
        2 => 2,
        3 => 5,
        _ => 16,
    };
    let lead_stride = if s.style == 0 || s.style == 2 { 2 } else { 4 };
    let step = spb / 4.0;
    let drive = s.drive;
    for bar in 0..BARS {
        let sec = section(bar);
        let bs = bar as f32 * 4.0 * spb;
        let fade = if sec == 4 { 1.0 - 0.5 * ((bar - 28) as f32 / 8.0) } else { 1.0 };
        let chord_pc = (s.root as i32 + prog[bar % 4]).rem_euclid(12);
        let q = qual[bar % 4];
        let soft = if s.style == 4 { 0.45 } else { 1.0 };
        if sec != 2 {
            for &st in &kick_steps {
                let acc = if st == 0 { 1.0 } else { 0.82 };
                kick(&mut buf, (bs + st as f32 * step) as usize, (0.55 + 0.4 * drive) * acc * fade * soft);
            }
            if sec != 0 || bar >= 4 {
                for &st in &clap_steps {
                    clap(&mut buf, &mut rng, (bs + st as f32 * step) as usize, (0.35 + 0.3 * drive) * fade);
                }
            }
        }
        for &st in &hat_steps {
            let mut amp = if sec == 2 { 0.03 } else { 0.05 + 0.06 * drive };
            if sec == 0 && bar < 2 {
                amp *= 0.7;
            }
            hat(&mut buf, &mut rng, (bs + st as f32 * step) as usize, amp * fade);
        }
        if sec == 1 || sec == 3 {
            let f = midi_freq(36.0 + chord_pc as f32);
            for &st in &bass_steps {
                let at = (bs + st as f32 * step) as usize;
                let len = (bass_len_steps as f32 * step) as usize;
                tone(&mut buf, at, f, len, 0.30, 100, 2.0, 0.35, 0.12);
            }
        }
        if sec == 1 || sec == 2 || sec == 3 || s.style == 4 {
            let gain = if sec == 2 { 1.4 } else { 1.0 } * if s.style == 4 { 1.8 } else { 1.0 };
            for tone_off in [0, q, 7] {
                let m = 48.0 + ((chord_pc + tone_off) % 12) as f32;
                let len = (4.0 * spb + 0.3 * sr) as usize;
                tone(&mut buf, bs as usize, midi_freq(m), len, 0.045 * gain * (0.6 + 0.4 * drive), (0.4 * sr) as usize, 0.0, 0.45, 0.0);
            }
        }
        if sec == 1 || sec == 3 || sec == 2 {
            let stride = if sec == 2 { 4 } else { lead_stride };
            let offs = [0, q, 7, 12, 7, q];
            let mut k = 0;
            let mut st = 0;
            while st < 16 {
                let m = 60.0 + chord_pc as f32 + offs[k % 6] as f32;
                let at = (bs + st as f32 * step) as usize;
                tone(&mut buf, at, midi_freq(m), (0.3 * sr) as usize, 0.05 + 0.04 * drive, 40, 8.0, 0.3, 0.0);
                k += 1;
                st += stride;
            }
        }
    }
    let peak = buf.iter().fold(0f32, |a, &b| a.max(b.abs())).max(1e-6);
    let k = 0.8 / peak;
    for v in buf.iter_mut() {
        *v *= k;
    }
    buf
}

// ----------------------------------------------------------------- analysis

#[derive(Clone)]
pub struct Analysis {
    pub bpm: f32,
    pub origin: f64,
    pub key: u8,
    pub minor: bool,
    pub key_conf: f32,
    pub raw_energy: f32,
    pub beat_strength: f32,
    pub intro_end: f64,
    pub outro_start: f64,
    pub gain: f32,
    pub vocal: f32,
    pub bright: f32,
    pub peaks: Vec<f32>,
    pub bars: Vec<[f32; 3]>,
    pub first_bar: usize,
    pub last_bar: usize,
    pub beat_conf: f32,
}

fn onset_env(pcm: &[f32]) -> Vec<f32> {
    let n = pcm.len() / HOP;
    let mut env = vec![0f32; n];
    for i in 0..n {
        let mut e = 0f32;
        let mut h = 0f32;
        for k in 0..HOP {
            let idx = i * HOP + k;
            let x = pcm[idx];
            let d = if idx > 0 { x - pcm[idx - 1] } else { x };
            e += x * x;
            h += d * d;
        }
        env[i] = (e / HOP as f32).sqrt() + 2.0 * (h / HOP as f32).sqrt();
    }
    let mut on = vec![0f32; n];
    for i in 1..n {
        on[i] = (env[i] - env[i - 1]).max(0.0);
    }
    on
}

/// Onset signal tuned for beat tracking: log-compressed (quiet drum hits count as much as
/// loud ones), local-average removed (sustained vocals/harmonium drop out), then lightly
/// smoothed so a live player's few-millisecond timing wobble still lines up on one grid.
fn onset_env_beat(pcm: &[f32]) -> Vec<f32> {
    let n = pcm.len() / HOP;
    let mut env = vec![0f32; n];
    for i in 0..n {
        let (mut e, mut h) = (0f32, 0f32);
        for k in 0..HOP {
            let idx = i * HOP + k;
            let x = pcm[idx];
            let d = if idx > 0 { x - pcm[idx - 1] } else { x };
            e += x * x;
            h += d * d;
        }
        env[i] = (e / HOP as f32).sqrt() + 2.0 * (h / HOP as f32).sqrt();
    }
    let mean = (env.iter().sum::<f32>() / n.max(1) as f32).max(1e-6);
    let lg: Vec<f32> = env.iter().map(|&v| (1.0 + 30.0 * v / mean).ln()).collect();
    let mut on = vec![0f32; n];
    for i in 1..n {
        on[i] = (lg[i] - lg[i - 1]).max(0.0);
    }
    // subtract a ~1 s moving average (prefix sums keep this O(n))
    let w = (SR as usize / HOP / 2).max(1);
    let mut pre = vec![0f64; n + 1];
    for i in 0..n {
        pre[i + 1] = pre[i] + on[i] as f64;
    }
    let mut out = vec![0f32; n];
    for i in 0..n {
        let (a, b) = (i.saturating_sub(w), (i + w + 1).min(n));
        let avg = ((pre[b] - pre[a]) / (b - a) as f64) as f32;
        out[i] = (on[i] - 1.2 * avg).max(0.0);
    }
    // 5-tap triangle smoothing (about 15 ms)
    let mut sm = vec![0f32; n];
    for i in 0..n {
        let g = |j: isize| -> f32 { if j >= 0 && (j as usize) < n { out[j as usize] } else { 0.0 } };
        let i = i as isize;
        sm[i as usize] = (g(i - 2) + 2.0 * g(i - 1) + 3.0 * g(i) + 2.0 * g(i + 1) + g(i + 2)) / 9.0;
    }
    sm
}

fn interp(on: &[f32], x: f32) -> f32 {
    if x < 0.0 {
        return 0.0;
    }
    let i = x as usize;
    if i + 1 >= on.len() {
        return 0.0;
    }
    let f = x - i as f32;
    on[i] * (1.0 - f) + on[i + 1] * f
}

fn ac(on: &[f32], p: f32) -> f32 {
    let mut s = 0f32;
    for i in 0..on.len() {
        if on[i] > 0.0 {
            s += on[i] * interp(on, i as f32 + p);
        }
    }
    s / on.len().max(1) as f32
}

fn comb(on: &[f32], phase: f32, p: f32) -> f32 {
    let mut s = 0f32;
    let mut c = 0;
    let mut x = phase.max(0.0);
    while x < (on.len() as f32 - 1.0) {
        s += interp(on, x);
        c += 1;
        x += p;
    }
    s / c.max(1) as f32
}

fn prior(b: f32) -> f32 {
    let l = (b / 115.0).log2() / 0.8;
    (-0.5 * l * l).exp()
}

pub fn detect_beats(pcm: &[f32]) -> (f32, f64, f32, f32) {
    detect_beats_with(pcm, true)
}

pub fn detect_beats_with(pcm: &[f32], tuned: bool) -> (f32, f64, f32, f32) {
    let on = if tuned { onset_env_beat(pcm) } else { onset_env(pcm) };
    let fps = SR as f32 / HOP as f32;
    let n_use = on.len().min(20000);
    let head = &on[..n_use];
    let mut best = (-1f32, 120f32);
    let mut b = 70.0f32;
    while b <= 175.0 {
        let p = 60.0 * fps / b;
        let s = (ac(head, p) + 0.5 * ac(head, 2.0 * p) + 0.25 * ac(head, 4.0 * p)) * prior(b);
        if s > best.0 {
            best = (s, b);
        }
        b += 0.25;
    }
    let b0 = best.1;
    let p0 = 60.0 * fps / b0;
    // coarse phase
    let mut ph0 = 0f32;
    let mut bs = -1f32;
    let mut ph = 0f32;
    let (mut csum, mut ccount) = (0f32, 0f32);
    while ph < p0 {
        let s = comb(&on, ph, p0);
        csum += s;
        ccount += 1.0;
        if s > bs {
            bs = s;
            ph0 = ph;
        }
        ph += 0.5;
    }
    // fine tempo + phase search over the whole track
    let mut fine = (-1f32, b0, ph0);
    let mut bf = b0 - 0.5;
    while bf <= b0 + 0.5 {
        let pf = 60.0 * fps / bf;
        let mut ph = ph0 - 2.0;
        while ph <= ph0 + 2.0 {
            let s = comb(&on, ph, pf);
            if s > fine.0 {
                fine = (s, bf, ph);
            }
            ph += 0.25;
        }
        bf += 0.01;
    }
    let bpm = fine.1;
    let pf = 60.0 * fps / bpm;
    let ph = fine.2.max(0.0);
    // downbeat: the beat of the bar with the strongest accent
    let mut bj = (-1f32, 0usize);
    for j in 0..4 {
        let mut s = 0f32;
        let mut k = j;
        while ph + (k as f32) * pf < on.len() as f32 - 1.0 {
            s += interp(&on, ph + k as f32 * pf);
            k += 4;
        }
        if s > bj.0 {
            bj = (s, j);
        }
    }
    let origin = (ph + bj.1 as f32 * pf) as f64 * HOP as f64 + HOP as f64 / 2.0;
    let strength = ac(head, 60.0 * fps / bpm) / ac(head, 0.0).max(1e-9);
    let conf = bs.max(0.0) / (csum / ccount.max(1.0)).max(1e-9);
    (bpm, origin, strength, conf)
}

fn goertzel(seg: &[f32], freq: f32) -> f32 {
    let w = 2.0 * PI * freq / SR as f32;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0f32, 0f32);
    for &x in seg {
        let s0 = x + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let p = s1 * s1 + s2 * s2 - c * s1 * s2;
    p.max(0.0).sqrt() / seg.len() as f32
}

pub fn detect_key(pcm: &[f32]) -> (u8, bool, f32) {
    let len = pcm.len();
    let seg_len = (24 * SR as usize).min(len);
    let start = (len / 4).min(len - seg_len);
    let seg = &pcm[start..start + seg_len];
    let mut chroma = [0f32; 12];
    for m in 36..96 {
        let f = midi_freq(m as f32);
        chroma[m % 12] += goertzel(seg, f);
    }
    let maj = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
    let min = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];
    let mc = chroma.iter().sum::<f32>() / 12.0;
    let mut scores: Vec<(f32, u8, bool)> = Vec::new();
    for root in 0..12 {
        for &is_min in &[false, true] {
            let prof = if is_min { &min } else { &maj };
            let mp = prof.iter().sum::<f32>() / 12.0;
            let (mut num, mut da, mut db) = (0f32, 0f32, 0f32);
            for i in 0..12 {
                let a = chroma[(root + i) % 12] - mc;
                let b = prof[i] - mp;
                num += a * b;
                da += a * a;
                db += b * b;
            }
            scores.push((num / (da * db).sqrt().max(1e-9), root as u8, is_min));
        }
    }
    scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    (scores[0].1, scores[0].2, (scores[0].0 - scores[1].0).max(0.0))
}

pub fn analyze(pcm: &[f32]) -> Analysis {
    let (bpm, origin_raw, strength, beat_conf) = detect_beats(pcm);
    let (key, minor, key_conf) = detect_key(pcm);
    let spb = SR as f64 * 60.0 / bpm as f64;
    let bar = 4.0 * spb;
    let mut origin = origin_raw % bar;
    if origin < 0.0 {
        origin += bar;
    }
    let nb = ((pcm.len() as f64 - origin) / bar).floor().max(1.0) as usize;
    let mut rms = Vec::with_capacity(nb);
    for b in 0..nb {
        let a = (origin + b as f64 * bar) as usize;
        let e = ((origin + (b + 1) as f64 * bar) as usize).min(pcm.len());
        let mut s = 0f32;
        for &x in &pcm[a.min(e)..e] {
            s += x * x;
        }
        rms.push((s / (e - a.min(e)).max(1) as f32).sqrt());
    }
    let mut sorted = rms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let level = sorted[((sorted.len() as f32 * 0.8) as usize).min(sorted.len() - 1)];
    let thr = 0.8 * level;
    let mut intro_bars = 0;
    for (i, &r) in rms.iter().enumerate() {
        if r >= thr {
            intro_bars = i;
            break;
        }
    }
    let mut outro_bars = nb;
    for i in (0..nb).rev() {
        if rms[i] >= thr {
            outro_bars = i + 1;
            break;
        }
    }
    if intro_bars > 16 {
        intro_bars = 8.min(nb);
    }
    if outro_bars < nb / 2 || outro_bars + 2 > nb {
        outro_bars = nb.saturating_sub(8).max(nb / 2);
    }
    let body: Vec<f32> = rms.iter().cloned().filter(|&r| r >= thr).collect();
    let body_rms = (body.iter().sum::<f32>() / body.len().max(1) as f32).max(1e-4);
    let total_rms = (pcm.iter().map(|x| x * x).sum::<f32>() / pcm.len() as f32).sqrt();
    // spectral brightness and vocal-range presence from one-pole splits
    let (mut l1, mut l2) = (0f32, 0f32);
    let (mut e_all, mut e_mid, mut e_hi) = (1e-9f32, 0f32, 0f32);
    let a1 = 1.0 - (-2.0 * PI * 300.0 / SR as f32).exp();
    let a2 = 1.0 - (-2.0 * PI * 3400.0 / SR as f32).exp();
    for &x in pcm.iter().step_by(2) {
        l1 += a1 * (x - l1);
        l2 += a2 * (x - l2);
        let mid = l2 - l1;
        let hi = x - l2;
        e_all += x * x;
        e_mid += mid * mid;
        e_hi += hi * hi;
    }
    // per-bar loudness / low-band / vocal-band profile for finding mix points
    let mut bars: Vec<[f32; 3]> = vec![[0.0; 3]; nb];
    let mut cnt: Vec<f32> = vec![0.0; nb];
    {
        let (mut f1, mut f2) = (0f32, 0f32);
        let lo = 1.0 - (-2.0 * PI * 250.0 / SR as f32).exp();
        for (idx, &x) in pcm.iter().enumerate() {
            f1 += lo * (x - f1);
            f2 += a2 * (x - f2);
            if (idx as f64) < origin {
                continue;
            }
            let b = ((idx as f64 - origin) / bar) as usize;
            if b >= nb {
                break;
            }
            let mid = f2 - a1 * 0.0 - f1;
            bars[b][0] += x * x;
            bars[b][1] += f1 * f1;
            bars[b][2] += mid * mid;
            cnt[b] += 1.0;
        }
        for b in 0..nb {
            let c = cnt[b].max(1.0);
            for k in 0..3 {
                bars[b][k] = (bars[b][k] / c).sqrt();
            }
        }
    }
    let silence = 0.08 * level;
    let mut first_bar = 0;
    for i in 0..nb {
        if bars[i][0] >= silence {
            first_bar = i;
            break;
        }
    }
    let mut last_bar = nb;
    for i in (0..nb).rev() {
        if bars[i][0] >= silence {
            last_bar = i + 1;
            break;
        }
    }
    let on = onset_env(pcm);
    let flux = on.iter().sum::<f32>() / on.len().max(1) as f32;
    // Waveform detail: loudness (RMS) per slice. Raw sample peaks sit near full scale on
    // mastered music, which makes the wave look flat. Rescale so the loud parts reach the
    // top and quiet parts (intro, breaks, outro) stay visibly lower.
    let step = (pcm.len() / 1536).max(1);
    let mut peaks: Vec<f32> = pcm
        .chunks(step)
        .map(|c| (c.iter().map(|&b| b * b).sum::<f32>() / c.len().max(1) as f32).sqrt())
        .collect();
    let mut sorted = peaks.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let top = sorted.get(sorted.len() * 98 / 100).copied().unwrap_or(1.0).max(1e-4);
    for v in peaks.iter_mut() {
        *v = (*v / top).min(1.0).powf(1.35);
    }
    Analysis {
        bpm,
        origin,
        key,
        minor,
        key_conf,
        raw_energy: total_rms * 2.0 + flux * 40.0 + (e_hi / e_all).sqrt(),
        beat_strength: strength,
        intro_end: origin + intro_bars as f64 * bar,
        outro_start: origin + outro_bars as f64 * bar,
        gain: (0.17 / body_rms).clamp(0.25, 4.0),
        vocal: (e_mid / e_all).sqrt(),
        bright: (e_hi / e_all).sqrt(),
        peaks,
        bars,
        first_bar,
        last_bar,
        beat_conf,
    }
}

// ------------------------------------------------------------------ library

#[derive(Clone)]
pub struct Track {
    pub id: usize,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub pcm: Arc<Vec<f32>>,
    pub a: Analysis,
    pub energy: f32,
    pub dance: f32,
    pub mood: String,
    pub hue: f32,
    pub true_bpm: Option<f32>,
    pub true_key: Option<(u8, bool)>,
    pub dur: f32,
    pub user: bool,
    /// Embedded cover art, decoded to RGBA (width, height, pixels).
    pub cover: Option<Arc<(u16, u16, Vec<u8>)>>,
    /// Source file and sample count for songs whose audio is loaded on demand.
    pub path: Option<std::path::PathBuf>,
    pub n: usize,
}

/// Decodes embedded picture bytes into a small RGBA thumbnail.
fn decode_cover(bytes: &[u8]) -> Option<Arc<(u16, u16, Vec<u8>)>> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = img.resize_to_fill(400, 400, image::imageops::FilterType::Triangle).to_rgba8();
    Some(Arc::new((img.width() as u16, img.height() as u16, img.into_raw())))
}

/// Dominant hue (degrees) of a cover, weighting vivid pixels most, like Apple Music's backdrop.
/// Returns None for grey/black-and-white covers.
fn cover_hue(px: &[u8]) -> Option<f32> {
    let (mut sx, mut sy, mut wsum) = (0f32, 0f32, 0f32);
    for p in px.chunks(4).step_by(7) {
        let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        let d = mx - mn;
        if d < 0.08 || mx < 0.12 {
            continue;
        }
        let h = if mx == r { ((g - b) / d).rem_euclid(6.0) } else if mx == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 } * 60.0;
        let wgt = (d / mx) * d; // saturation x chroma
        sx += wgt * h.to_radians().cos();
        sy += wgt * h.to_radians().sin();
        wsum += wgt;
    }
    if wsum < 1.0 {
        return None;
    }
    Some(sy.atan2(sx).to_degrees().rem_euclid(360.0))
}

pub const NOTE: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

pub fn key_name(k: u8, minor: bool) -> String {
    format!("{} {}", NOTE[k as usize % 12], if minor { "min" } else { "maj" })
}

/// Camelot wheel number (1..12) and letter (A minor / B major).
pub fn camelot(k: u8, minor: bool) -> (i32, char) {
    let major_pc = if minor { (k as i32 + 3) % 12 } else { k as i32 };
    let n = (major_pc * 7 + 7).rem_euclid(12) + 1;
    (n, if minor { 'A' } else { 'B' })
}

pub fn camelot_str(k: u8, minor: bool) -> String {
    let (n, l) = camelot(k, minor);
    format!("{}{}", n, l)
}

fn load_wav(path: &std::path::Path) -> Option<Vec<f32>> {
    let d = std::fs::read(path).ok()?;
    if d.len() < 44 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
        return None;
    }
    let (mut fmt, mut ch, mut sr, mut bits) = (0u16, 0usize, 0u32, 0u16);
    let mut data: Option<&[u8]> = None;
    let mut p = 12;
    while p + 8 <= d.len() {
        let id = &d[p..p + 4];
        let sz = u32::from_le_bytes([d[p + 4], d[p + 5], d[p + 6], d[p + 7]]) as usize;
        let body = &d[p + 8..(p + 8 + sz).min(d.len())];
        if id == b"fmt " && body.len() >= 16 {
            fmt = u16::from_le_bytes([body[0], body[1]]);
            ch = u16::from_le_bytes([body[2], body[3]]) as usize;
            sr = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            data = Some(body);
        }
        p += 8 + sz + (sz & 1);
    }
    let data = data?;
    if ch == 0 || sr == 0 {
        return None;
    }
    let bytes = (bits / 8) as usize;
    if bytes == 0 {
        return None;
    }
    let frames = data.len() / (bytes * ch);
    let mut mono = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut acc = 0f32;
        for c in 0..ch {
            let o = (f * ch + c) * bytes;
            let v = match (fmt, bits) {
                (1, 16) => i16::from_le_bytes([data[o], data[o + 1]]) as f32 / 32768.0,
                (1, 24) => (((data[o] as i32) << 8 | (data[o + 1] as i32) << 16 | (data[o + 2] as i32) << 24) >> 8) as f32 / 8388608.0,
                (3, 32) => f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]),
                _ => return None,
            };
            acc += v;
        }
        mono.push(acc / ch as f32);
    }
    Some(resample(&mono, sr))
}

fn resample(mono: &[f32], sr: u32) -> Vec<f32> {
    let ratio = sr as f64 / SR as f64;
    let n = (mono.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let x = i as f64 * ratio;
        let j = x as usize;
        let f = (x - j as f64) as f32;
        let a = mono[j];
        let b = *mono.get(j + 1).unwrap_or(&a);
        out.push(a * (1.0 - f) + b * f);
    }
    out
}


/// Folders searched for music files (next to the program and the working folder).
pub fn music_dirs() -> Vec<std::path::PathBuf> {
    let mut v = vec![std::path::PathBuf::from("music")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent() {
            v.push(p.join("music"));
            if let Some(pp) = p.parent().and_then(|x| x.parent()) {
                v.push(pp.join("music"));
            }
        }
    }
    v
}

pub static CHOSEN_DIR: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

pub fn set_music_dir(p: std::path::PathBuf) {
    let _ = std::fs::write("music_folder.txt", p.to_string_lossy().as_bytes());
    *CHOSEN_DIR.lock().unwrap() = Some(p);
}

pub fn load_saved_music_dir() {
    if let Ok(s) = std::fs::read_to_string("music_folder.txt") {
        let p = std::path::PathBuf::from(s.trim());
        if p.is_dir() {
            *CHOSEN_DIR.lock().unwrap() = Some(p);
        }
    }
}

fn collect_audio(dir: &std::path::Path, depth: u32, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 3 {
                    collect_audio(&p, depth + 1, out);
                }
            } else {
                let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                if ["mp3", "wav", "flac", "ogg"].contains(&ext.as_str()) {
                    out.push(p);
                }
            }
        }
    }
}

pub fn primary_music_dir() -> std::path::PathBuf {
    if let Some(p) = CHOSEN_DIR.lock().unwrap().clone() {
        return p;
    }
    for d in music_dirs() {
        if d.is_dir() {
            return std::fs::canonicalize(&d).unwrap_or(d);
        }
    }
    let d = std::path::PathBuf::from("music");
    let _ = std::fs::create_dir_all(&d);
    std::fs::canonicalize(&d).unwrap_or(d)
}

fn mood_for(energy: f32, minor: bool) -> String {
    let m = if energy > 0.7 {
        if minor { "Intense" } else { "Euphoric" }
    } else if energy < 0.4 {
        if minor { "Melancholic" } else { "Warm" }
    } else if minor {
        "Dark groove"
    } else {
        "Upbeat groove"
    };
    m.to_string()
}

/// Recomputes ids and the library-relative energy / danceability / mood.
pub fn finalize(tracks: &mut Vec<Track>) {
    let (mut emin, mut emax, mut dmin, mut dmax) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for t in tracks.iter() {
        emin = emin.min(t.a.raw_energy);
        emax = emax.max(t.a.raw_energy);
        dmin = dmin.min(t.a.beat_strength);
        dmax = dmax.max(t.a.beat_strength);
    }
    for (i, t) in tracks.iter_mut().enumerate() {
        t.id = i;
        let a = &t.a;
        t.energy = if emax > emin { 0.12 + 0.83 * (a.raw_energy - emin) / (emax - emin) } else { 0.5 };
        let bs = if dmax > dmin { (a.beat_strength - dmin) / (dmax - dmin) } else { 0.5 };
        let tempo_fit = 1.0 - ((a.bpm - 120.0).abs() / 60.0).min(1.0);
        t.dance = (0.3 + 0.35 * bs + 0.35 * tempo_fit).clamp(0.0, 1.0);
        t.mood = mood_for(t.energy, a.minor);
    }
}

fn make_track(spec: Option<&Spec>, title: String, artist: String, album: String, genre: String, pcm: Vec<f32>) -> Track {
    let a = analyze(&pcm);
    let dur = pcm.len() as f32 / SR as f32;
    let hue = match spec {
        Some(s) => s.hue,
        None => (title.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)) % 360) as f32,
    };
    Track {
        id: 0,
        title,
        artist,
        album,
        genre,
        pcm: Arc::new(pcm),
        a,
        energy: 0.5,
        dance: 0.5,
        mood: String::new(),
        hue,
        true_bpm: spec.map(|s| s.bpm),
        true_key: spec.map(|s| (s.root, s.minor)),
        dur,
        user: spec.is_none(),
        cover: None,
        path: None,
        n: 0,
    }
}

/// Synthesised demo tracks (rendered and analysed in parallel).
pub fn build_demo() -> Vec<Track> {
    let specs = specs();
    let mut tracks: Vec<Track> = std::thread::scope(|sc| {
        let hs: Vec<_> = specs
            .iter()
            .map(|s| {
                sc.spawn(move || {
                    let pcm = render(s);
                    make_track(Some(s), s.title.to_string(), s.artist.to_string(), s.album.to_string(), s.genre.to_string(), pcm)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    finalize(&mut tracks);
    tracks
}

// ------------------------------------------------- background music scanning

#[derive(Default)]
pub struct Progress {
    pub phase: String,
    pub current: String,
    pub found: usize,
    pub done: usize,
    pub loaded: usize,
    pub failed: Vec<String>,
    pub finished: bool,
    pub result: Option<Vec<Track>>,
    pub folder: String,
}

struct Decoded {
    pcm: Vec<f32>,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    genre: Option<String>,
    cover: Option<Vec<u8>>,
}

fn decode_file(path: &std::path::Path) -> Result<Decoded, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("unsupported file ({})", e))?;
    let mut tags: [Option<String>; 4] = [None, None, None, None];
    let mut cover: Option<Vec<u8>> = None;
    fn read(rev: &MetadataRevision, tags: &mut [Option<String>; 4], cover: &mut Option<Vec<u8>>) {
        if cover.is_none() {
            if let Some(v) = rev.visuals().first() {
                *cover = Some(v.data.to_vec());
            }
        }
        for t in rev.tags() {
            let v = t.value.to_string();
            if v.trim().is_empty() {
                continue;
            }
            match t.std_key {
                Some(StandardTagKey::TrackTitle) => tags[0] = Some(v),
                Some(StandardTagKey::Artist) => tags[1] = Some(v),
                Some(StandardTagKey::Album) => tags[2] = Some(v),
                Some(StandardTagKey::Genre) => tags[3] = Some(v),
                _ => {}
            }
        }
    }
    if let Some(md) = probed.metadata.get() {
        if let Some(rev) = md.current() {
            read(rev, &mut tags, &mut cover);
        }
    }
    let mut format = probed.format;
    if let Some(rev) = format.metadata().current() {
        read(rev, &mut tags, &mut cover);
    }
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or("no audio track")?;
    let params = track.codec_params.clone();
    let id = track.id;
    let sr = params.sample_rate.ok_or("unknown sample rate")?;
    let mut decoder = symphonia::default::get_codecs().make(&params, &DecoderOptions::default()).map_err(|e| format!("cannot decode ({})", e))?;
    let mut mono: Vec<f32> = Vec::new();
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    let mut cap = 0usize;
    let limit = sr as usize * 60 * 9;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let ch = buf.spec().channels.count().max(1);
                if sbuf.is_none() || buf.capacity() > cap {
                    cap = buf.capacity();
                    sbuf = Some(SampleBuffer::<f32>::new(cap as u64, *buf.spec()));
                }
                let sb = sbuf.as_mut().unwrap();
                sb.copy_interleaved_ref(buf);
                for fr in sb.samples().chunks(ch) {
                    mono.push(fr.iter().sum::<f32>() / ch as f32);
                }
                if mono.len() > limit {
                    break;
                }
            }
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }
    if mono.len() < sr as usize * 8 {
        return Err("shorter than 8 seconds or empty".to_string());
    }
    let [title, artist, album, genre] = tags;
    Ok(Decoded { pcm: resample(&mono, sr), title, artist, album, genre, cover })
}

/// Starts scanning `dir` in the background; progress is reported through `prog`.
pub fn start_scan(dir: std::path::PathBuf, prog: Arc<std::sync::Mutex<Progress>>) {
    std::thread::spawn(move || {
        {
            let mut p = prog.lock().unwrap();
            p.phase = "Looking for songs".to_string();
            p.folder = dir.to_string_lossy().to_string();
        }
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        collect_audio(&dir, 0, &mut files);
        files.sort();
        files.truncate(400);
        {
            let mut p = prog.lock().unwrap();
            p.found = files.len();
            p.phase = "Analysing".to_string();
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let out: std::sync::Mutex<Vec<(usize, Track)>> = std::sync::Mutex::new(Vec::new());
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 4);
        std::thread::scope(|sc| {
            for _ in 0..workers {
                sc.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if i >= files.len() {
                        break;
                    }
                    let path = &files[i];
                    let fname = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    prog.lock().unwrap().current = fname.clone();
                    match decode_file(path) {
                        Ok(d) => {
                            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                            let album = d.album.unwrap_or_else(|| path.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Your music".to_string()));
                            let cover = d.cover.as_deref().and_then(decode_cover);
                            let mut t = make_track(None, d.title.unwrap_or(stem), d.artist.unwrap_or_else(|| "Unknown artist".to_string()), album, d.genre.unwrap_or_else(|| "Unknown".to_string()), d.pcm);
                            if let Some(h) = cover.as_ref().and_then(|c| cover_hue(&c.2)) {
                                t.hue = h;
                            }
                            t.cover = cover;
                            t.n = t.pcm.len();
                            t.path = Some(path.clone());
                            t.pcm = Arc::new(Vec::new()); // free the audio; it is re-decoded when played
                            out.lock().unwrap().push((i, t));
                            let mut p = prog.lock().unwrap();
                            p.done += 1;
                            p.loaded += 1;
                        }
                        Err(e) => {
                            let mut p = prog.lock().unwrap();
                            p.done += 1;
                            p.failed.push(format!("{}: {}", fname, e));
                        }
                    }
                });
            }
        });
        let mut v = out.into_inner().unwrap();
        v.sort_by_key(|x| x.0);
        let mut p = prog.lock().unwrap();
        p.result = Some(v.into_iter().map(|x| x.1).collect());
        p.finished = true;
        p.phase = "Done".to_string();
    });
}

/// Looks for a folder with songs next to the program: `music` or `reference`
/// in the program's folder or any folder above it (and the current folder).
pub fn find_default_music() -> Option<std::path::PathBuf> {
    let mut bases: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        bases.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent() {
            bases.push(p.to_path_buf());
        }
    }
    for b in bases {
        let mut cur: Option<&std::path::Path> = Some(b.as_path());
        let mut depth = 0;
        while let Some(dir) = cur {
            for name in ["music", "reference"] {
                let cand = dir.join(name);
                if cand.is_dir() {
                    let mut files = Vec::new();
                    collect_audio(&cand, 0, &mut files);
                    if !files.is_empty() {
                        return Some(cand);
                    }
                }
            }
            cur = dir.parent();
            depth += 1;
            if depth > 6 {
                break;
            }
        }
    }
    None
}

// ------------------------------------------------- on-demand audio
// Scanned songs keep only their analysis in memory; the decoded audio is dropped and
// re-decoded when a song is about to play (a small cache keeps the last few).

#[derive(Default)]
struct AudioCache {
    map: std::collections::HashMap<std::path::PathBuf, Arc<Vec<f32>>>,
    order: Vec<std::path::PathBuf>,
    loading: std::collections::HashSet<std::path::PathBuf>,
}

fn audio_cache() -> &'static std::sync::Mutex<AudioCache> {
    static C: std::sync::OnceLock<std::sync::Mutex<AudioCache>> = std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(AudioCache::default()))
}

const CACHE_KEEP: usize = 4;

fn cache_put(p: &std::path::Path, a: Arc<Vec<f32>>) {
    let mut c = audio_cache().lock().unwrap();
    c.loading.remove(p);
    c.order.retain(|q| q != p);
    c.order.push(p.to_path_buf());
    c.map.insert(p.to_path_buf(), a);
    while c.order.len() > CACHE_KEEP {
        let old = c.order.remove(0);
        c.map.remove(&old);
    }
}

impl Track {
    /// Number of samples (works whether or not the audio is currently in memory).
    pub fn samples(&self) -> usize {
        if self.path.is_some() {
            self.n
        } else {
            self.pcm.len()
        }
    }

    /// The decoded audio. Demo songs always have it; scanned songs are decoded on demand.
    pub fn audio(&self) -> Arc<Vec<f32>> {
        let p = match &self.path {
            None => return self.pcm.clone(),
            Some(p) => p,
        };
        if let Some(a) = audio_cache().lock().unwrap().map.get(p).cloned() {
            return a;
        }
        let a = Arc::new(decode_file(p).map(|d| d.pcm).unwrap_or_else(|_| vec![0.0; self.n]));
        cache_put(p, a.clone());
        a
    }

    /// Starts decoding in the background so `audio()` is instant when the song is needed.
    pub fn prefetch(&self) {
        let p = match &self.path {
            None => return,
            Some(p) => p.clone(),
        };
        {
            let mut c = audio_cache().lock().unwrap();
            if c.map.contains_key(&p) || c.loading.contains(&p) {
                return;
            }
            c.loading.insert(p.clone());
        }
        let n = self.n;
        std::thread::spawn(move || {
            let a = Arc::new(decode_file(&p).map(|d| d.pcm).unwrap_or_else(|_| vec![0.0; n]));
            cache_put(&p, a);
        });
    }
}
