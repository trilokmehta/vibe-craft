//! Native inverse STFT for the end of HT-Demucs (what the model's part B does).
//!
//! The exported model does its inverse STFT as two huge ConvTranspose ops, which
//! take ~1.5 s per 7.8 s window on the CPU. This is the same maths with an FFT
//! (torch.istft, n_fft 4096, hop 1024, Hann window, normalized, center) followed by
//! HT-Demucs' trim and the add of the time branch: a few milliseconds per window.

use crate::stems::Fft;

pub const NFFT: usize = 4096;
pub const HOP: usize = 1024;

/// `spec`: [S, C, 2 (re, im), F = NFFT/2+1, T] already padded (as /Pad_2).
/// `xt`: time branch [S, C, len] (as /Add_4). Returns stems [S, C, len].
pub fn part_b(spec: &[f32], xt: &[f32], s: usize, c: usize, frames: usize, len: usize) -> Vec<f32> {
    let f = NFFT / 2 + 1;
    let win: Vec<f32> = (0..NFFT).map(|k| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * k as f32 / NFFT as f32).cos()).collect();
    // HT-Demucs: pad = hop/2*3, le = hop*ceil(len/hop); istft(length = le + 2*pad); keep [pad, pad+len).
    let pad = HOP / 2 * 3;
    let le = HOP * ((len + HOP - 1) / HOP);
    let full = (frames - 1) * HOP + NFFT;
    let scale = (NFFT as f32).sqrt();
    let mut env = vec![0f32; full];
    for t in 0..frames {
        for k in 0..NFFT {
            env[t * HOP + k] += win[k] * win[k];
        }
    }
    let n_ch = s * c;
    let mut out = vec![0f32; n_ch * len];
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(n_ch).max(1);
    let per = (n_ch + workers - 1) / workers;
    std::thread::scope(|sc| {
        for (wi, chunk) in out.chunks_mut(per * len).enumerate() {
            let (win, env) = (&win, &env);
            sc.spawn(move || {
                let fft = Fft::new(NFFT);
                let mut re = vec![0f32; NFFT];
                let mut im = vec![0f32; NFFT];
                let mut acc = vec![0f32; full];
                for (j, o) in chunk.chunks_mut(len).enumerate() {
                    let ch = wi * per + j;
                    let base = ch * 2 * f * frames;
                    acc.iter_mut().for_each(|v| *v = 0.0);
                    for t in 0..frames {
                        // Hermitian spectrum -> real frame. DC and Nyquist imaginary parts are ignored (as irfft).
                        for b in 0..f {
                            re[b] = spec[base + b * frames + t];
                            im[b] = spec[base + f * frames + b * frames + t];
                        }
                        im[0] = 0.0;
                        im[f - 1] = 0.0;
                        for b in 1..f - 1 {
                            re[NFFT - b] = re[b];
                            im[NFFT - b] = -im[b];
                        }
                        fft.run(&mut re, &mut im, true);
                        let a = &mut acc[t * HOP..t * HOP + NFFT];
                        for k in 0..NFFT {
                            a[k] += re[k] * scale * win[k];
                        }
                    }
                    // center=True drops NFFT/2 at the start; then HT-Demucs drops `pad`.
                    let off = NFFT / 2 + pad;
                    let xt_ch = &xt[ch * len..(ch + 1) * len];
                    for i in 0..len {
                        let p = off + i;
                        let e = env[p];
                        let v = if e > 1e-11 { acc[p] / e } else { 0.0 };
                        o[i] = v + xt_ch[i];
                    }
                    let _ = le;
                }
            });
        }
    });
    out
}
