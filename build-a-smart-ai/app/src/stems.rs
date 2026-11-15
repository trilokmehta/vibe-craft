//! Offline layer separation (bass / drums / vocals-and-lead / music) using
//! signal processing only: harmonic-percussive separation on a spectrogram,
//! a low-frequency band for bass, and a "changing vs steady" mask inside the
//! vocal band. The four layers always add back up to the original song.
use crate::tracks::SR;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub const NL: usize = 5;
pub const NAMES: [&str; NL] = ["Vocals", "Music", "Bass", "Drums", "Melody"];

pub struct Stems {
    pub data: [Arc<Vec<f32>>; NL],
    /// True when made by the AI model (has a real Melody layer).
    pub ai: bool,
}

pub(crate) struct Fft {
    n: usize,
    rev: Vec<usize>,
    cs: Vec<f32>,
    sn: Vec<f32>,
}

impl Fft {
    pub(crate) fn new(n: usize) -> Fft {
        let bits = n.trailing_zeros();
        let rev = (0..n).map(|i| (i.reverse_bits() >> (usize::BITS - bits)) as usize).collect();
        let cs = (0..n / 2).map(|k| (2.0 * PI * k as f32 / n as f32).cos()).collect();
        let sn = (0..n / 2).map(|k| -(2.0 * PI * k as f32 / n as f32).sin()).collect();
        Fft { n, rev, cs, sn }
    }

    pub(crate) fn run(&self, re: &mut [f32], im: &mut [f32], inverse: bool) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i];
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let step = n / len;
            let mut i = 0;
            while i < n {
                for j in 0..half {
                    let wr = self.cs[j * step];
                    let wi = if inverse { -self.sn[j * step] } else { self.sn[j * step] };
                    let (ur, ui) = (re[i + j], im[i + j]);
                    let (xr, xi) = (re[i + j + half], im[i + j + half]);
                    let vr = xr * wr - xi * wi;
                    let vi = xr * wi + xi * wr;
                    re[i + j] = ur + vr;
                    im[i + j] = ui + vi;
                    re[i + j + half] = ur - vr;
                    im[i + j + half] = ui - vi;
                }
                i += len;
            }
            len <<= 1;
        }
        if inverse {
            let s = 1.0 / n as f32;
            for i in 0..n {
                re[i] *= s;
                im[i] *= s;
            }
        }
    }
}

fn median(buf: &mut [f32]) -> f32 {
    let m = buf.len() / 2;
    let (_, v, _) = buf.select_nth_unstable_by(m, |a, b| a.partial_cmp(b).unwrap());
    *v
}

const N: usize = 2048;
const H: usize = 512;
const B: usize = N / 2 + 1;
const CH: usize = 192;
const MARGIN: usize = 24;

/// Splits a song into [vocals, music, bass, drums]. `progress` goes 0..100.
pub fn separate(pcm: &[f32], progress: &AtomicU32) -> Stems {
    let len = pcm.len();
    let mut padded = vec![0f32; N + len + 2 * N];
    padded[N..N + len].copy_from_slice(pcm);
    let total_frames = (padded.len() - N) / H + 1;
    let fft = Fft::new(N);
    let win: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / N as f32).cos()).collect();
    let mut out: [Vec<f32>; 4] = [vec![0f32; padded.len()], vec![0f32; padded.len()], vec![0f32; padded.len()], vec![0f32; padded.len()]];
    let hz = SR as f32 / N as f32;
    let bass_bin = (180.0 / hz) as usize;
    let voc_lo = (200.0 / hz) as usize;
    let voc_hi = ((4500.0 / hz) as usize).min(B - 1);
    let mut c0 = 0;
    while c0 < total_frames {
        let c1 = (c0 + CH).min(total_frames);
        let l0 = c0.saturating_sub(MARGIN);
        let l1 = (c1 + MARGIN).min(total_frames);
        let nl = l1 - l0;
        let mut re = vec![0f32; nl * B];
        let mut im = vec![0f32; nl * B];
        let mut mag = vec![0f32; nl * B];
        let mut fr = vec![0f32; N];
        let mut fi = vec![0f32; N];
        for t in 0..nl {
            let s = (l0 + t) * H;
            for i in 0..N {
                fr[i] = padded[s + i] * win[i];
                fi[i] = 0.0;
            }
            fft.run(&mut fr, &mut fi, false);
            for k in 0..B {
                re[t * B + k] = fr[k];
                im[t * B + k] = fi[k];
                mag[t * B + k] = (fr[k] * fr[k] + fi[k] * fi[k]).sqrt();
            }
        }
        // running sums over time for the "steady" estimate
        let mut pref = vec![0f32; (nl + 1) * B];
        for t in 0..nl {
            for k in 0..B {
                pref[(t + 1) * B + k] = pref[t * B + k] + mag[t * B + k];
            }
        }
        let mut sp: [Vec<f32>; 4] = [vec![0f32; B * 2], vec![0f32; B * 2], vec![0f32; B * 2], vec![0f32; B * 2]];
        let _ = &mut sp;
        for t in c0..c1 {
            let lt = t - l0;
            let mut specs_re: [Vec<f32>; 4] = [vec![0f32; N], vec![0f32; N], vec![0f32; N], vec![0f32; N]];
            let mut specs_im: [Vec<f32>; 4] = [vec![0f32; N], vec![0f32; N], vec![0f32; N], vec![0f32; N]];
            let ta = lt.saturating_sub(8);
            let tb = (lt + 8).min(nl - 1);
            let sa = lt.saturating_sub(MARGIN);
            let sb = (lt + MARGIN + 1).min(nl);
            let mut buf = [0f32; 17];
            for k in 0..B {
                let mut n = 0;
                for tt in ta..=tb {
                    buf[n] = mag[tt * B + k];
                    n += 1;
                }
                let hm = median(&mut buf[..n]);
                let ka = k.saturating_sub(8);
                let kb = (k + 8).min(B - 1);
                let mut n2 = 0;
                for kk in ka..=kb {
                    buf[n2] = mag[lt * B + kk];
                    n2 += 1;
                }
                let pm = median(&mut buf[..n2]);
                let h2 = hm * hm;
                let mh = h2 / (h2 + pm * pm + 1e-12);
                let mp = 1.0 - mh;
                let steady = (pref[sb * B + k] - pref[sa * B + k]) / (sb - sa) as f32;
                let y = mag[lt * B + k];
                let (bass, vocal) = if k < bass_bin {
                    (mh, 0.0)
                } else if k >= voc_lo && k <= voc_hi {
                    let mv = ((y - steady) / (y + 1e-9)).clamp(0.0, 1.0);
                    (0.0, mh * mv)
                } else {
                    (0.0, 0.0)
                };
                let music = (mh - bass - vocal).max(0.0);
                let w = [vocal, music, bass, mp];
                let (xr, xi) = (re[lt * B + k], im[lt * B + k]);
                for s in 0..4 {
                    specs_re[s][k] = xr * w[s];
                    specs_im[s][k] = xi * w[s];
                    if k > 0 && k < N / 2 {
                        specs_re[s][N - k] = xr * w[s];
                        specs_im[s][N - k] = -xi * w[s];
                    }
                }
            }
            // two real signals per inverse transform: z = a + i*b
            for pair in 0..2 {
                let (a, b) = (pair * 2, pair * 2 + 1);
                for k in 0..N {
                    fr[k] = specs_re[a][k] - specs_im[b][k];
                    fi[k] = specs_im[a][k] + specs_re[b][k];
                }
                fft.run(&mut fr, &mut fi, true);
                let s = t * H;
                for i in 0..N {
                    out[a][s + i] += fr[i] * win[i];
                    out[b][s + i] += fi[i] * win[i];
                }
            }
        }
        progress.store((c1 * 100 / total_frames) as u32, Ordering::Relaxed);
        c0 = c1;
    }
    let norm = 1.0 / 1.5;
    let mk = |v: &Vec<f32>| -> Arc<Vec<f32>> { Arc::new(v[N..N + len].iter().map(|x| x * norm).collect()) };
    progress.store(100, Ordering::Relaxed);
    Stems { data: [mk(&out[0]), mk(&out[1]), mk(&out[2]), mk(&out[3]), Arc::new(vec![0.0; len])], ai: false }
}
