//! Trainer: finds where each downloaded song plays inside the DJ mix and learns
//! the DJ's transition habits from it.
//!   trainer match      -> results/matches.tsv   (snippet fingerprints of every song located in the mix)
//!   trainer segments   -> results/segments.tsv  (contiguous stretches of each song inside the mix)
//!   trainer fit        -> results/fit.txt       (statistics + learned transition weights)
#[path = "../../../src/tracks.rs"]
#[allow(dead_code, unused)]
mod tracks;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub const SR: usize = 22050;
const N: usize = 2048;
const NB: usize = 24;
const FPS: f64 = SR as f64 / N as f64;
const L: usize = 48; // snippet length in frames (~4.5 s)
const STRIDE: usize = 96;

fn root() -> PathBuf {
    let mut p = std::env::current_dir().unwrap();
    // run from "training 1" or from "training 1/trainer"
    if p.ends_with("trainer") {
        p.pop();
    }
    p
}

// ------------------------------------------------------------------ decoding

fn decode(path: &Path) -> Vec<f32> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    let file = std::fs::File::open(path).unwrap();
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).expect("probe");
    let mut format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).expect("track");
    let params = track.codec_params.clone();
    let id = track.id;
    let sr = params.sample_rate.expect("sr") as usize;
    let mut dec = symphonia::default::get_codecs().make(&params, &DecoderOptions::default()).expect("codec");
    let mut mono: Vec<f32> = Vec::new();
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    let mut cap = 0usize;
    while let Ok(pk) = format.next_packet() {
        if pk.track_id() != id {
            continue;
        }
        if let Ok(buf) = dec.decode(&pk) {
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
        }
    }
    // area resample to SR
    let ratio = sr as f64 / SR as f64;
    let n = (mono.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let a = (i as f64 * ratio) as usize;
        let b = (((i + 1) as f64 * ratio) as usize).max(a + 1).min(mono.len());
        let a = a.min(b.saturating_sub(1));
        out.push(mono[a..b].iter().sum::<f32>() / (b - a) as f32);
    }
    out
}

// ------------------------------------------------------------------ features

fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let (mut cr, mut ci) = (1f32, 0f32);
            for k in 0..len / 2 {
                let (ur, ui) = (re[i + k], im[i + k]);
                let (vr, vi) = (re[i + k + len / 2] * cr - im[i + k + len / 2] * ci, re[i + k + len / 2] * ci + im[i + k + len / 2] * cr);
                re[i + k] = ur + vr;
                im[i + k] = ui + vi;
                re[i + k + len / 2] = ur - vr;
                im[i + k + len / 2] = ui - vi;
                let t = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = t;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// Per-frame spectral fingerprint: log band energies, band-mean and frame-mean removed, unit norm.
fn features(pcm: &[f32]) -> Vec<f32> {
    let mel = |f: f32| 2595.0 * (1.0 + f / 700.0).log10();
    let imel = |m: f32| 700.0 * (10f32.powf(m / 2595.0) - 1.0);
    let (m0, m1) = (mel(80.0), mel(8000.0));
    let mut edges = Vec::new();
    for b in 0..=NB {
        let f = imel(m0 + (m1 - m0) * b as f32 / NB as f32);
        edges.push(((f * N as f32 / SR as f32) as usize).clamp(1, N / 2 - 1));
    }
    for b in 1..=NB {
        if edges[b] <= edges[b - 1] {
            edges[b] = edges[b - 1] + 1;
        }
    }
    let win: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
    let nf = pcm.len() / N;
    let mut out = vec![0f32; nf * NB];
    let (mut re, mut im) = (vec![0f32; N], vec![0f32; N]);
    for f in 0..nf {
        for i in 0..N {
            re[i] = pcm[f * N + i] * win[i];
            im[i] = 0.0;
        }
        fft(&mut re, &mut im);
        for b in 0..NB {
            let mut e = 0f32;
            for k in edges[b]..edges[b + 1] {
                e += re[k] * re[k] + im[k] * im[k];
            }
            e /= (edges[b + 1] - edges[b]) as f32;
            out[f * NB + b] = (e.sqrt() / N as f32 + 1e-5).ln();
        }
    }
    let mut mean = [0f32; NB];
    for f in 0..nf {
        for b in 0..NB {
            mean[b] += out[f * NB + b] / nf.max(1) as f32;
        }
    }
    for f in 0..nf {
        let fr = &mut out[f * NB..(f + 1) * NB];
        for b in 0..NB {
            fr[b] -= mean[b];
        }
        let m = fr.iter().sum::<f32>() / NB as f32;
        let mut n2 = 0f32;
        for v in fr.iter_mut() {
            *v -= m;
            n2 += *v * *v;
        }
        let n = n2.sqrt().max(1e-6);
        for v in fr.iter_mut() {
            *v /= n;
        }
    }
    out
}

fn songs_list(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|e| e == "mp3").unwrap_or(false)).collect();
    v.retain(|p| !p.to_string_lossy().contains("Signal Addition")); // wrong video (signals lecture)
    v.sort();
    v
}

fn short_title(p: &Path) -> String {
    let s = p.file_stem().unwrap().to_string_lossy().to_string();
    s.chars().take(60).collect()
}

// ------------------------------------------------------------------ stage 1: match

#[derive(Clone, Debug)]
struct Match {
    song: usize,
    s0: usize,
    p: usize,
    r: f64,
    score: f32,
    z: f32,
}

fn stage_match() {
    let root = root();
    let t0 = std::time::Instant::now();
    eprintln!("decoding mix...");
    let mix_pcm = decode(&mix_path(&root));
    let mix = features(&mix_pcm);
    let nmf = mix.len() / NB;
    eprintln!("mix: {:.1} min, {} frames ({:.0}s)", mix_pcm.len() as f64 / SR as f64 / 60.0, nmf, t0.elapsed().as_secs_f32());
    drop(mix_pcm);
    let files = songs_list(&songs_dir(&root));
    let ratios: Vec<f64> = (0..21).map(|i| 0.85 + 0.02 * i as f64).collect();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Match>> = Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for _ in 0..11 {
            sc.spawn(|| loop {
                let si = next.fetch_add(1, Ordering::SeqCst);
                if si >= files.len() {
                    break;
                }
                let pcm = decode(&files[si]);
                let song = features(&pcm);
                let nsf = song.len() / NB;
                let mut local: Vec<Match> = Vec::new();
                let mut s0 = 8;
                while s0 + L < nsf {
                    let mut best: Option<Match> = None;
                    for &r in &ratios {
                        if (s0 as f64 + (L + 1) as f64 * r) as usize + 1 >= nsf {
                            continue;
                        }
                        // stretched snippet
                        let mut snip = vec![0f32; L * NB];
                        for i in 0..L {
                            let x = s0 as f64 + i as f64 * r;
                            let j = x as usize;
                            let f = (x - j as f64) as f32;
                            for b in 0..NB {
                                snip[i * NB + b] = song[j * NB + b] * (1.0 - f) + song[(j + 1) * NB + b] * f;
                            }
                        }
                        let (mut bs, mut bp) = (f32::MIN, 0usize);
                        let (mut sum, mut sq, mut cnt) = (0f64, 0f64, 0f64);
                        for p in 0..nmf - L {
                            let w = &mix[p * NB..(p + L) * NB];
                            let mut d = 0f32;
                            for k in 0..L * NB {
                                d += snip[k] * w[k];
                            }
                            let d = d / L as f32;
                            sum += d as f64;
                            sq += (d as f64) * (d as f64);
                            cnt += 1.0;
                            if d > bs {
                                bs = d;
                                bp = p;
                            }
                        }
                        let m = sum / cnt;
                        let sd = ((sq / cnt - m * m).max(1e-12)).sqrt();
                        let z = ((bs as f64 - m) / sd) as f32;
                        if best.as_ref().map(|b| z > b.z).unwrap_or(true) {
                            best = Some(Match { song: si, s0, p: bp, r, score: bs, z });
                        }
                    }
                    if let Some(b) = best {
                        local.push(b);
                    }
                    s0 += STRIDE;
                }
                eprintln!("[{:>2}/{}] {} ({} snippets, {:.0}s)", si + 1, files.len(), short_title(&files[si]), local.len(), t0.elapsed().as_secs_f32());
                results.lock().unwrap().extend(local);
            });
        }
    });
    let mut all = results.into_inner().unwrap();
    all.sort_by(|a, b| (a.song, a.s0).cmp(&(b.song, b.s0)));
    let mut s = String::from("song\ts0\tp\tr\tscore\tz\n");
    for m in &all {
        s.push_str(&format!("{}\t{}\t{}\t{:.3}\t{:.4}\t{:.2}\n", m.song, m.s0, m.p, m.r, m.score, m.z));
    }
    std::fs::write(root.join("results/matches.tsv"), s).unwrap();
    let mut names = String::new();
    for (i, f) in files.iter().enumerate() {
        names.push_str(&format!("{}\t{}\n", i, f.file_stem().unwrap().to_string_lossy()));
    }
    std::fs::write(root.join("results/songs.tsv"), names).unwrap();
    eprintln!("done in {:.0}s, {} matches", t0.elapsed().as_secs_f32(), all.len());
}

// ------------------------------------------------------------------ stage 2: segments

#[derive(Clone, Debug)]
struct Seg {
    song: usize,
    mix_start: f64, // seconds
    mix_end: f64,
    song_start: f64,
    song_end: f64,
    r: f64,
    n: usize,
    z: f32,
}

fn read_matches(root: &Path) -> Vec<Match> {
    let t = std::fs::read_to_string(root.join("results/matches.tsv")).unwrap();
    t.lines()
        .skip(1)
        .map(|l| {
            let p: Vec<&str> = l.split('\t').collect();
            Match { song: p[0].parse().unwrap(), s0: p[1].parse().unwrap(), p: p[2].parse().unwrap(), r: p[3].parse().unwrap(), score: p[4].parse().unwrap(), z: p[5].parse().unwrap() }
        })
        .collect()
}

fn read_names(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("results/songs.tsv")).unwrap().lines().map(|l| l.split('\t').nth(1).unwrap().to_string()).collect()
}

fn stage_segments(zt: f32) {
    let root = root();
    let matches = read_matches(&root);
    let names = read_names(&root);
    eprintln!("z distribution: {:?}", {
        let mut z: Vec<f32> = matches.iter().map(|m| m.z).collect();
        z.sort_by(|a, b| a.partial_cmp(b).unwrap());
        [0.1, 0.25, 0.5, 0.75, 0.9, 0.97].map(|q| z[((z.len() - 1) as f32 * q) as usize])
    });
    let mix_pcm = decode(&mix_path(&root));
    let mix = features(&mix_pcm);
    let nmf = mix.len() / NB;
    let files = songs_list(&songs_dir(&root));
    let mut segs: Vec<Seg> = Vec::new();
    for si in 0..files.len() {
        let mut ms: Vec<&Match> = matches.iter().filter(|m| m.song == si && m.z >= zt).collect();
        ms.sort_by_key(|m| m.s0);
        if ms.is_empty() {
            eprintln!("NO MATCH: {}", names[si]);
            continue;
        }
        // cluster by constant offset a = p - s0 / r
        let mut clusters: Vec<Vec<&Match>> = Vec::new();
        for m in ms {
            let a = m.p as f64 - m.s0 as f64 / m.r;
            let mut placed = false;
            for c in clusters.iter_mut() {
                let l = c.last().unwrap();
                let la = l.p as f64 - l.s0 as f64 / l.r;
                if (a - la).abs() < 10.0 && (m.r - l.r).abs() < 0.04 {
                    c.push(m);
                    placed = true;
                    break;
                }
            }
            if !placed {
                clusters.push(vec![m]);
            }
        }
        let pcm = decode(&files[si]);
        let song = features(&pcm);
        let nsf = song.len() / NB;
        for c in clusters {
            let strong = c.iter().map(|m| m.z).fold(0f32, f32::max);
            if c.len() < 2 && strong < zt * 1.6 {
                continue;
            }
            let mut rs: Vec<f64> = c.iter().map(|m| m.r).collect();
            rs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let r = rs[rs.len() / 2];
            let mut aa: Vec<f64> = c.iter().map(|m| m.p as f64 - m.s0 as f64 / r).collect();
            aa.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let a = aa[aa.len() / 2];
            let anchor = c.iter().map(|m| m.score).sum::<f32>() / c.len() as f32;
            // per-song-frame similarity along the line
            let mut sim = vec![f32::NAN; nsf];
            for t in 0..nsf {
                let mf = a + t as f64 / r;
                if mf < 0.0 || mf as usize >= nmf {
                    continue;
                }
                let mf = mf.round() as usize;
                if mf >= nmf {
                    continue;
                }
                let mut d = 0f32;
                for b in 0..NB {
                    d += song[t * NB + b] * mix[mf * NB + b];
                }
                sim[t] = d;
            }
            let sm = |t: usize| -> f32 {
                let (lo, hi) = (t.saturating_sub(8), (t + 9).min(nsf));
                let v: Vec<f32> = sim[lo..hi].iter().cloned().filter(|x| !x.is_nan()).collect();
                if v.len() < 4 { -1.0 } else { v.iter().sum::<f32>() / v.len() as f32 }
            };
            let thr = 0.35 * anchor;
            let first = c.first().unwrap().s0;
            let last = c.last().unwrap().s0 + L;
            let mut lo = first;
            while lo > 0 && sm(lo - 1) > thr {
                lo -= 1;
            }
            let mut hi = last.min(nsf - 1);
            while hi + 1 < nsf && sm(hi + 1) > thr {
                hi += 1;
            }
            let song_start = lo as f64 / FPS;
            let song_end = hi as f64 / FPS;
            segs.push(Seg {
                song: si,
                mix_start: (a + lo as f64 / r) / FPS,
                mix_end: (a + hi as f64 / r) / FPS,
                song_start,
                song_end,
                r,
                n: c.len(),
                z: c.iter().map(|m| m.z).sum::<f32>() / c.len() as f32,
            });
        }
    }
    segs.sort_by(|a, b| a.mix_start.partial_cmp(&b.mix_start).unwrap());
    let mut s = String::from("song\tname\tmix_start\tmix_end\tsong_start\tsong_end\trate\tn\tz\n");
    for g in &segs {
        s.push_str(&format!("{}\t{}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.3}\t{}\t{:.1}\n", g.song, names[g.song].chars().take(40).collect::<String>().replace('\t', " "), g.mix_start, g.mix_end, g.song_start, g.song_end, g.r, g.n, g.z));
    }
    std::fs::write(root.join("results/segments.tsv"), &s).unwrap();
    eprintln!("{} segments, mix length {:.0}s", segs.len(), nmf as f64 / FPS);
}

fn read_segments(root: &Path) -> Vec<Seg> {
    let t = std::fs::read_to_string(root.join("results/segments.tsv")).unwrap();
    t.lines()
        .skip(1)
        .map(|l| {
            let p: Vec<&str> = l.split('\t').collect();
            Seg { song: p[0].parse().unwrap(), mix_start: p[2].parse().unwrap(), mix_end: p[3].parse().unwrap(), song_start: p[4].parse().unwrap(), song_end: p[5].parse().unwrap(), r: p[6].parse().unwrap(), n: p[7].parse().unwrap(), z: p[8].parse().unwrap() }
        })
        .collect()
}

// ------------------------------------------------------------------ stage 3: fit

struct Info {
    a: tracks::Analysis,
    dur: f64,
    energy: f32,
    dance: f32,
}

fn key_score(a: &Info, b: &Info) -> f32 {
    let (na, la) = tracks::camelot(a.a.key, a.a.minor);
    let (nb, lb) = tracks::camelot(b.a.key, b.a.minor);
    let d = {
        let x = (na - nb).abs();
        x.min(12 - x)
    };
    let low = a.a.key_conf < 0.02 || b.a.key_conf < 0.02;
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
    if low { 0.5 * s + 0.25 } else { s }
}

fn tempo_score(out_eff: f32, bpm_b: f32, range: f64) -> f32 {
    let r0 = out_eff as f64 / bpm_b as f64;
    let mut best = r0;
    for m in [0.5f64, 2.0] {
        if (r0 * m).ln().abs() < best.ln().abs() {
            best = r0 * m;
        }
    }
    (1.0 - best.ln().abs() / range).clamp(0.0, 1.0) as f32
}

/// feature vector for "B follows A": tempo, key, energy closeness, dance closeness
fn feats(a: &Info, bpm_a_eff: f32, b: &Info) -> [f32; 4] {
    [
        tempo_score(bpm_a_eff, b.a.bpm, 0.16),
        key_score(a, b),
        (1.0 - (b.energy - a.energy).abs() * 2.5).clamp(0.0, 1.0),
        1.0 - (b.dance - a.dance).abs(),
    ]
}

fn pct(v: &mut Vec<f64>, q: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * q) as usize]
}

fn stage_fit() {
    let root = root();
    let segs = read_segments(&root);
    let names = read_names(&root);
    let files = songs_list(&songs_dir(&root));
    let by_order = std::env::var("FITORDER").is_ok(); // tracklist order known: files are numbered in play order
    let used: Vec<usize> = if by_order {
        (0..files.len()).collect()
    } else {
        let mut u: Vec<usize> = segs.iter().map(|s| s.song).collect();
        u.sort();
        u.dedup();
        u
    };
    let infos: Mutex<Vec<Option<Info>>> = Mutex::new((0..files.len()).map(|_| None).collect());
    let next = AtomicUsize::new(0);
    std::thread::scope(|sc| {
        for _ in 0..11 {
            sc.spawn(|| loop {
                let k = next.fetch_add(1, Ordering::SeqCst);
                if k >= used.len() {
                    break;
                }
                let si = used[k];
                let pcm = decode(&files[si]);
                let a = tracks::analyze(&pcm);
                let dur = pcm.len() as f64 / SR as f64;
                infos.lock().unwrap()[si] = Some(Info { a, dur, energy: 0.0, dance: 0.0 });
            });
        }
    });
    let mut infos = infos.into_inner().unwrap();
    // library-relative energy / danceability, same formula as tracks::finalize
    let (mut emin, mut emax, mut dmin, mut dmax) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for i in infos.iter().flatten() {
        emin = emin.min(i.a.raw_energy);
        emax = emax.max(i.a.raw_energy);
        dmin = dmin.min(i.a.beat_strength);
        dmax = dmax.max(i.a.beat_strength);
    }
    for i in infos.iter_mut().flatten() {
        i.energy = if emax > emin { 0.12 + 0.83 * (i.a.raw_energy - emin) / (emax - emin) } else { 0.5 };
        let bs = if dmax > dmin { (i.a.beat_strength - dmin) / (dmax - dmin) } else { 0.5 };
        let fit = 1.0 - ((i.a.bpm - 120.0).abs() / 60.0).min(1.0);
        i.dance = (0.3 + 0.35 * bs + 0.35 * fit).clamp(0.0, 1.0);
    }
    let mut out = String::new();
    macro_rules! say {
        ($($t:tt)*) => {{ let l = format!($($t)*); println!("{}", l); out.push_str(&l); out.push('\n'); }};
    }
    // main timeline = one segment per song (the longest), in mix order
    let mut main: Vec<Seg> = Vec::new();
    for &si in &used {
        if let Some(best) = segs.iter().filter(|s| s.song == si).max_by(|a, b| (a.mix_end - a.mix_start).partial_cmp(&(b.mix_end - b.mix_start)).unwrap()) {
            main.push(best.clone());
        }
    }
    main.sort_by(|a, b| a.mix_start.partial_cmp(&b.mix_start).unwrap());
    say!("== timeline ({} songs placed in the mix) ==", main.len());
    for g in &main {
        let i = infos[g.song].as_ref().unwrap();
        say!(
            "{:>7.1}-{:>7.1}s  {:<40} bpm {:>5.1} x{:.3}  {}  E{:.2}  plays song {:.0}-{:.0}s of {:.0}s",
            g.mix_start, g.mix_end, names[g.song].chars().take(40).collect::<String>(), i.a.bpm, g.r, tracks::camelot_str(i.a.key, i.a.minor), i.energy, g.song_start, g.song_end, i.dur
        );
    }
    // ---------- habits
    let mut play: Vec<f64> = Vec::new();
    let mut frac: Vec<f64> = Vec::new();
    let mut shift: Vec<f64> = Vec::new();
    let mut ovl: Vec<f64> = Vec::new();
    let mut ovl_beats: Vec<f64> = Vec::new();
    let mut bpm_step: Vec<f64> = Vec::new();
    let mut eff_step: Vec<f64> = Vec::new();
    let mut ent_frac: Vec<f64> = Vec::new();
    let mut at_open = 0;
    let mut kd: Vec<f64> = Vec::new();
    for g in &main {
        let i = infos[g.song].as_ref().unwrap();
        play.push(g.mix_end - g.mix_start);
        frac.push((g.song_end - g.song_start) / i.dur);
        shift.push((g.r - 1.0).abs() * 100.0);
        ent_frac.push(g.song_start / i.dur);
        let open = i.a.origin / SR as f64 + i.a.first_bar as f64 * 4.0 * 60.0 / i.a.bpm as f64;
        if g.song_start < open + 6.0 {
            at_open += 1;
        }
    }
    for w in main.windows(2) {
        let (ga, gb) = (&w[0], &w[1]);
        let (ia, ib) = (infos[ga.song].as_ref().unwrap(), infos[gb.song].as_ref().unwrap());
        if gb.mix_start - ga.mix_end >= 12.0 {
            continue;
        }
        let o = ga.mix_end - gb.mix_start;
        ovl.push(o);
        let eff_a = ia.a.bpm as f64 * ga.r;
        ovl_beats.push(o * eff_a / 60.0);
        bpm_step.push((ib.a.bpm as f64 / ia.a.bpm as f64 - 1.0).abs() * 100.0);
        eff_step.push(((ib.a.bpm as f64 * gb.r) / eff_a - 1.0).abs() * 100.0);
        let (na, _) = tracks::camelot(ia.a.key, ia.a.minor);
        let (nb, _) = tracks::camelot(ib.a.key, ib.a.minor);
        let d = (na - nb).abs();
        kd.push(d.min(12 - d) as f64);
    }
    say!("\n== habits ==");
    say!("seconds each song plays   p10/50/90: {:.0} / {:.0} / {:.0}", pct(&mut play.clone(), 0.1), pct(&mut play.clone(), 0.5), pct(&mut play.clone(), 0.9));
    say!("fraction of song played   p10/50/90: {:.2} / {:.2} / {:.2}", pct(&mut frac.clone(), 0.1), pct(&mut frac.clone(), 0.5), pct(&mut frac.clone(), 0.9));
    say!("entry point in song (frac of length) p10/50/90: {:.2} / {:.2} / {:.2}; entered at the opening: {}/{}", pct(&mut ent_frac.clone(), 0.1), pct(&mut ent_frac.clone(), 0.5), pct(&mut ent_frac.clone(), 0.9), at_open, main.len());
    say!("tempo shift applied to songs (%) p50/90/max: {:.1} / {:.1} / {:.1}", pct(&mut shift.clone(), 0.5), pct(&mut shift.clone(), 0.9), pct(&mut shift.clone(), 1.0));
    say!("raw BPM step between consecutive songs (%) p50/90/max: {:.1} / {:.1} / {:.1}", pct(&mut bpm_step.clone(), 0.5), pct(&mut bpm_step.clone(), 0.9), pct(&mut bpm_step.clone(), 1.0));
    say!("effective BPM step after DJ shifting (%) p50/90/max: {:.1} / {:.1} / {:.1}", pct(&mut eff_step.clone(), 0.5), pct(&mut eff_step.clone(), 0.9), pct(&mut eff_step.clone(), 1.0));
    say!("overlap length (s) p10/50/90: {:.1} / {:.1} / {:.1}", pct(&mut ovl.clone(), 0.1), pct(&mut ovl.clone(), 0.5), pct(&mut ovl.clone(), 0.9));
    say!("overlap length (beats) p10/50/90: {:.1} / {:.1} / {:.1}", pct(&mut ovl_beats.clone(), 0.1), pct(&mut ovl_beats.clone(), 0.5), pct(&mut ovl_beats.clone(), 0.9));
    let mut kh = [0usize; 7];
    for d in &kd {
        kh[(*d as usize).min(6)] += 1;
    }
    say!("Camelot number distance between consecutive songs (0..6): {:?}", kh);
    // ---------- learn transition weights (conditional logit over songs not yet played)
    let order: Vec<usize> = if by_order { (0..files.len()).collect() } else { main.iter().map(|g| g.song).collect() };
    let rate: Vec<f64> = if by_order {
        order.iter().map(|&si| main.iter().find(|g| g.song == si).map(|g| g.r).unwrap_or(1.0)).collect()
    } else {
        main.iter().map(|g| g.r).collect()
    };
    let mut cases: Vec<(Vec<[f32; 4]>, usize)> = Vec::new(); // candidate feats, index of the true one
    for k in 0..order.len() - 1 {
        if !by_order && main[k + 1].mix_start - main[k].mix_end >= 12.0 {
            continue;
        }
        let a = infos[order[k]].as_ref().unwrap();
        let eff = a.a.bpm * rate[k] as f32;
        let mut cand: Vec<[f32; 4]> = Vec::new();
        let mut truth = 0;
        for j in k + 1..order.len() {
            let b = infos[order[j]].as_ref().unwrap();
            if j == k + 1 {
                truth = cand.len();
            }
            cand.push(feats(a, eff, b));
        }
        if cand.len() >= 3 {
            cases.push((cand, truth));
        }
    }
    let eval = |w: &[f32; 4]| -> (f64, f64, f64, f64) {
        let (mut ll, mut top1, mut top3, mut rk) = (0f64, 0f64, 0f64, 0f64);
        for (c, t) in &cases {
            let sc: Vec<f32> = c.iter().map(|f| (0..4).map(|i| w[i] * f[i]).sum()).collect();
            let m = sc.iter().cloned().fold(f32::MIN, f32::max);
            let z: f32 = sc.iter().map(|s| (s - m).exp()).sum();
            ll += ((sc[*t] - m) - z.ln()) as f64;
            let rank = sc.iter().filter(|&&s| s > sc[*t]).count();
            let eq = (sc.iter().filter(|&&s| s == sc[*t]).count() - 1) as f64;
            if rank == 0 { top1 += 1.0 / (eq + 1.0); }
            if rank < 3 { top3 += ((3 - rank) as f64 / (eq + 1.0)).min(1.0); }
            rk += (rank as f64 + eq / 2.0) / (c.len() - 1) as f64;
        }
        let n = cases.len() as f64;
        (ll / n, top1 / n, top3 / n, rk / n)
    };
    let mut w = [0f32; 4];
    for _ in 0..4000 {
        let mut g = [0f32; 4];
        for (c, t) in &cases {
            let sc: Vec<f32> = c.iter().map(|f| (0..4).map(|i| w[i] * f[i]).sum()).collect();
            let m = sc.iter().cloned().fold(f32::MIN, f32::max);
            let ex: Vec<f32> = sc.iter().map(|s| (s - m).exp()).collect();
            let z: f32 = ex.iter().sum();
            for i in 0..4 {
                let e: f32 = c.iter().zip(&ex).map(|(f, p)| f[i] * p / z).sum();
                g[i] += c[*t][i] - e;
            }
        }
        for i in 0..4 {
            w[i] += 0.02 * (g[i] / cases.len() as f32 - 0.02 * w[i]);
        }
    }
    let old = [2.2f32, 1.6, 1.4, 0.5];
    let none = [0f32; 4];
    let (a0, b0, c0, d0) = eval(&none);
    let (a1, b1, c1, d1) = eval(&old);
    let (a2, b2, c2, d2) = eval(&w);
    say!("\n== learned transition weights over {} real transitions ==", cases.len());
    say!("features: tempo-fit, key-fit, energy-closeness, dance-closeness");
    say!("learned (raw):       {:.3?}", w);
    let so: f32 = old.iter().sum();
    let sw: f32 = w.iter().map(|x| x.max(0.0)).sum();
    let wn: Vec<f32> = w.iter().map(|x| x.max(0.0) / sw * so).collect();
    say!("learned (scaled to the engine's current total {:.1}): {:.2?}", so, wn);
    say!("existing engine weights: {:?}", old);
    say!("                        loglik   top1   top3   mean-rank(0=best, 0.5=random)");
    say!("random order            {:.3}   {:.2}   {:.2}   {:.2}", a0, b0, c0, d0);
    say!("existing weights        {:.3}   {:.2}   {:.2}   {:.2}", a1, b1, c1, d1);
    say!("learned weights         {:.3}   {:.2}   {:.2}   {:.2}", a2, b2, c2, d2);
    // tempo range actually used
    say!("tempo-fit range check: songs shifted up to {:.1}% in the mix", pct(&mut shift.clone(), 1.0));
    std::fs::write(root.join("results/fit.txt"), out).unwrap();
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("match") => stage_match(),
        Some("segments") => stage_segments(std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8.0)),
        Some("fit") => stage_fit(),
        _ => eprintln!("usage: trainer match|segments|fit"),
    }
}

fn mix_path(root: &Path) -> PathBuf {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|e| e == "mp3").unwrap_or(false) && p.file_name().unwrap().to_string_lossy().starts_with("main")).collect();
    v.sort();
    v.remove(0)
}

fn songs_dir(root: &Path) -> PathBuf {
    if root.join("songs").is_dir() { root.join("songs") } else { root.join("tracks") }
}
