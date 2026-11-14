//! Phase 2: "where does the vibe continue?"
//!
//! From a song's AI layers we build a per-bar profile (how loud each layer is,
//! and which notes are sounding). AutoMix then looks for an exit in the playing
//! song and an entry in the next one so that:
//!   * the outgoing vocals have finished (or are about to) when the blend starts,
//!   * the incoming vocals only arrive once the blend is over,
//!   * the incoming song brings beats first (drums present at its entry),
//!   * what the next song sounds like after the blend matches what the
//!     current one sounded like before it (energy, layer balance, notes/key).

use crate::stems::{Stems, NL};
use crate::tracks::SR;

const V: usize = 0; // vocals
const MU: usize = 1; // music
const BA: usize = 2; // bass
const DR: usize = 3; // drums
const ME: usize = 4; // melody

pub struct Profile {
    pub origin: f64,
    pub bar: f64,
    /// RMS of each layer per bar, divided by the song's typical bar level.
    pub lv: Vec<[f32; NL]>,
    /// Layer "is playing" per bar (above a fraction of that layer's usual level).
    pub on: Vec<[bool; NL]>,
    /// Pitch-class profile per bar from the harmonic layers (normalised).
    pub chroma: Vec<[f32; 12]>,
}

fn goertzel(x: &[f32], sr: f32, f: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * f / sr;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0f32, 0f32);
    for &v in x {
        let s0 = v + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0)
}

pub fn profile(stems: &Stems, origin: f64, bar: f64, nbars: usize) -> Profile {
    let n = stems.data[0].len();
    let mut lv = vec![[0f32; NL]; nbars];
    let mut chroma = vec![[0f32; 12]; nbars];
    for b in 0..nbars {
        let s = (origin + b as f64 * bar).max(0.0) as usize;
        let e = ((origin + (b + 1) as f64 * bar) as usize).min(n);
        if e <= s + 64 {
            continue;
        }
        for k in 0..NL {
            let d = &stems.data[k][s..e];
            lv[b][k] = (d.iter().map(|x| x * x).sum::<f32>() / d.len() as f32).sqrt();
        }
        // Notes: harmonic layers only (no drums), decimated 4x to ~5.5 kHz.
        let h: Vec<f32> = (s..e).step_by(4).map(|i| stems.data[V][i] * 0.5 + stems.data[MU][i] + stems.data[ME][i] + stems.data[BA][i] * 0.5).collect();
        let hsr = SR as f32 / 4.0;
        // Use a 0.5 s slice from the middle of the bar to keep it cheap.
        let mid = h.len() / 2;
        let half = ((0.25 * hsr) as usize).min(mid);
        let seg = &h[mid - half..(mid + half).min(h.len())];
        for pc in 0..12 {
            let mut acc = 0f32;
            for oct in 3..7 {
                let midi = 12 * (oct + 1) + pc as i32;
                let f = 440.0 * 2f32.powf((midi - 69) as f32 / 12.0);
                if f < hsr * 0.45 {
                    acc += goertzel(seg, hsr, f).sqrt();
                }
            }
            chroma[b][pc] = acc;
        }
        let m = chroma[b].iter().cloned().fold(0.0, f32::max);
        if m > 0.0 {
            chroma[b].iter_mut().for_each(|c| *c /= m);
        }
    }
    // Normalise by the song's typical (60th percentile) total bar level.
    let mut tot: Vec<f32> = lv.iter().map(|l| l.iter().map(|x| x * x).sum::<f32>().sqrt()).filter(|x| *x > 1e-4).collect();
    tot.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let body = if tot.is_empty() { 1.0 } else { tot[(tot.len() * 6 / 10).min(tot.len() - 1)].max(1e-4) };
    lv.iter_mut().for_each(|l| l.iter_mut().for_each(|x| *x /= body));
    // A layer is "on" in a bar if it is above 30% of its own loud level.
    let mut on = vec![[false; NL]; nbars];
    for k in 0..NL {
        let mut v: Vec<f32> = lv.iter().map(|l| l[k]).filter(|x| *x > 1e-3).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let loud = if v.is_empty() { f32::MAX } else { v[(v.len() * 8 / 10).min(v.len() - 1)] };
        for b in 0..nbars {
            on[b][k] = lv[b][k] > 0.3 * loud && lv[b][k] > 0.02;
        }
    }
    Profile { origin, bar, lv, on, chroma }
}

impl Profile {
    fn bar_at(&self, pos: f64) -> isize {
        ((pos - self.origin) / self.bar).round() as isize
    }

    fn frac_on(&self, k: usize, a: isize, b: isize) -> f32 {
        let lo = a.max(0) as usize;
        let hi = (b.max(0) as usize).min(self.on.len());
        if hi <= lo {
            return 0.0;
        }
        self.on[lo..hi].iter().filter(|o| o[k]).count() as f32 / (hi - lo) as f32
    }

    /// Mean layer levels + mean chroma over bars a..b.
    fn window(&self, a: isize, b: isize) -> Option<([f32; NL], [f32; 12])> {
        let lo = a.max(0) as usize;
        let hi = (b.max(0) as usize).min(self.lv.len());
        if hi <= lo {
            return None;
        }
        let mut l = [0f32; NL];
        let mut c = [0f32; 12];
        for i in lo..hi {
            for k in 0..NL {
                l[k] += self.lv[i][k];
            }
            for p in 0..12 {
                c[p] += self.chroma[i][p];
            }
        }
        let n = (hi - lo) as f32;
        l.iter_mut().for_each(|x| *x /= n);
        c.iter_mut().for_each(|x| *x /= n);
        Some((l, c))
    }
}

fn cos(a: &[f32], b: &[f32]) -> f32 {
    let d: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na < 1e-6 || nb < 1e-6 {
        0.5
    } else {
        d / (na * nb)
    }
}

/// Best transposition match of two chroma vectors without shifting (keys are not
/// pitch-shifted yet): plain cosine, so only genuinely compatible notes score high.
fn note_match(a: &[f32; 12], b: &[f32; 12]) -> f32 {
    cos(a, b)
}

/// Extra score for leaving song A at `out_pos` with a blend `t_bars` long.
/// Returns (score, reason).
pub fn out_bonus(p: &Profile, out_pos: f64, t_bars: f64) -> (f32, &'static str) {
    let i = p.bar_at(out_pos);
    let t = t_bars.ceil() as isize;
    let voc_before = p.frac_on(V, i - 4, i);
    let voc_during = p.frac_on(V, i, i + t);
    // Best: vocals have just finished a line and the blend runs over an instrumental part.
    let mut s = 0.9 * (1.0 - voc_during);
    let mut why = "instrumental stretch";
    if voc_before > 0.5 && voc_during < 0.2 {
        s += 0.3;
        why = "right after a vocal line ends";
    } else if voc_during > 0.6 {
        why = "vocals still singing";
    }
    (s, why)
}

/// Extra score for entering song B at `in_pos` with a blend `t_bars` long.
pub fn in_bonus(p: &Profile, in_pos: f64, t_bars: f64) -> (f32, &'static str) {
    let j = p.bar_at(in_pos);
    let t = t_bars.ceil() as isize;
    let voc_during = p.frac_on(V, j, j + t);
    let voc_after = p.frac_on(V, j + t, j + t + 4);
    let drums_first = p.frac_on(DR, j, j + (t / 2).max(1));
    // Beats should lead in; vocals should wait until the blend is done, then arrive.
    let mut s = 0.8 * (1.0 - voc_during) + 0.5 * drums_first;
    let mut why = if drums_first > 0.5 { "beats lead in" } else { "soft entry" };
    if voc_during < 0.2 && voc_after > 0.5 {
        s += 0.3;
        why = "vocals arrive right after the blend";
    } else if voc_during > 0.6 {
        why = "vocals would overlap";
    }
    (s, why)
}

/// How well the next song, once it has taken over, continues the current song's feel.
/// 0..1, plus a short description.
pub fn continuity(pa: &Profile, out_pos: f64, pb: &Profile, in_pos: f64, t_bars: f64) -> (f32, String) {
    let i = pa.bar_at(out_pos);
    let j = pb.bar_at(in_pos) + t_bars.ceil() as isize;
    let (Some((la, ca)), Some((lb, cb))) = (pa.window(i - 4, i), pb.window(j, j + 4)) else {
        return (0.5, String::new());
    };
    let ea: f32 = la.iter().map(|x| x * x).sum::<f32>().sqrt();
    let eb: f32 = lb.iter().map(|x| x * x).sum::<f32>().sqrt();
    let energy = 1.0 - ((ea - eb).abs() / ea.max(eb).max(0.05)).min(1.0);
    // Layer balance without vocals (vocals are handled by the hand-off rules).
    let ia = [la[MU], la[BA], la[DR], la[ME]];
    let ib = [lb[MU], lb[BA], lb[DR], lb[ME]];
    let balance = cos(&ia, &ib).max(0.0);
    let notes = note_match(&ca, &cb).max(0.0);
    let s = 0.4 * energy + 0.3 * balance + 0.3 * notes;
    (s, format!("vibe match {:.0}% (energy {:.0}%, layers {:.0}%, notes {:.0}%)", s * 100.0, energy * 100.0, balance * 100.0, notes * 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn tone(n: usize, f: f32, amp: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * std::f32::consts::PI * f * i as f32 / SR as f32).sin()).collect()
    }

    #[test]
    fn chroma_finds_a() {
        let n = SR as usize * 8;
        let a = Arc::new(tone(n, 440.0, 0.3));
        let z = Arc::new(vec![0f32; n]);
        let s = Stems { data: [z.clone(), a, z.clone(), z.clone(), z], ai: true };
        let p = profile(&s, 0.0, SR as f64 * 2.0, 4);
        let c = p.chroma[1];
        let best = (0..12).max_by(|x, y| c[*x].partial_cmp(&c[*y]).unwrap()).unwrap();
        assert_eq!(best, 9, "A is pitch class 9, got {:?}", c);
        assert!(p.on[1][MU] && !p.on[1][V]);
    }
}

/// `--vibe-test [n]`: separates the first n songs (default 3) of the music folder
/// and prints, for each consecutive pair, the mix AutoMix picks with and without
/// the layer profile, plus the vocal overlap each plan would cause.
pub fn self_test(lib: &[crate::tracks::Track], n: usize) {
    use crate::dj::{choose_mix, MixOpts};
    crate::sep::ensure_model();
    while matches!(crate::sep::model_state(), crate::sep::ModelState::Downloading(_)) {
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let mut profs = Vec::new();
    for t in lib.iter().take(n) {
        let Some(path) = &t.path else { continue };
        let pcm = t.audio();
        let prog = std::sync::atomic::AtomicU32::new(0);
        let t0 = std::time::Instant::now();
        match crate::sep::separate_file(path, &pcm, &prog) {
            Ok(s) => {
                let bar = 4.0 * SR as f64 * 60.0 / t.a.bpm.max(40.0) as f64;
                profs.push((t, profile(&s, t.a.origin, bar, t.a.bars.len())));
                eprintln!("separated {} in {:.0} s", t.title, t0.elapsed().as_secs_f32());
            }
            Err(e) => eprintln!("skip {}: {}", t.title, e),
        }
    }
    let overlap = |pa: &Profile, o: f64, pb: &Profile, i: f64, tb: f64| -> f32 {
        // bars of the blend where both songs have vocals on
        let (a0, b0) = (pa.bar_at(o), pb.bar_at(i));
        (0..tb as isize).filter(|k| pa.frac_on(V, a0 + k, a0 + k + 1) > 0.0 && pb.frac_on(V, b0 + k, b0 + k + 1) > 0.0).count() as f32
    };
    let (mut ov_old, mut ov_new, mut c_old, mut c_new) = (0f32, 0f32, 0f32, 0f32);
    for w in profs.windows(2) {
        let ((ta, pa), (tb, pb)) = (&w[0], &w[1]);
        let tbars = 4.0;
        let old = choose_mix(ta, tb, 0.0, 1.0, 16, false, MixOpts::default(), None);
        let new = choose_mix(ta, tb, 0.0, 1.0, 16, false, MixOpts::default(), Some((pa, pb)));
        println!("\n{} -> {}", ta.title, tb.title);
        for (label, c) in [("old", &old), ("new", &new)] {
            if let Some(c) = c {
                let ov = overlap(pa, c.out_pos, pb, c.in_pos, tbars);
                let (cv, _) = continuity(pa, c.out_pos, pb, c.in_pos, tbars);
                println!("  {}: {}\n       vocal-overlap bars {:.0}/4, vibe match {:.0}%", label, c.why, ov, cv * 100.0);
                if label == "old" { ov_old += ov; c_old += cv; } else { ov_new += ov; c_new += cv; }
            }
        }
    }
    let pairs = profs.len().saturating_sub(1).max(1) as f32;
    println!("\nTOTAL over {} pairs: vocal-overlap bars old {:.0} new {:.0}; mean vibe match old {:.0}% new {:.0}%", pairs, ov_old, ov_new, 100.0 * c_old / pairs, 100.0 * c_new / pairs);
}
