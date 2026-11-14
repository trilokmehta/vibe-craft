//! AI layer separation with HT-Demucs 6-stem (MIT licence) running on ONNX Runtime.
//!
//! The model (~136 MB) is downloaded once into %LOCALAPPDATA%\VibeCraft\models.
//! Songs are decoded again from their file in stereo at full quality, split in
//! 7.8 s windows with 25% overlap, and the six model stems are folded into the
//! app's layers:  Vocals | Music (other + residual) | Bass | Drums | Melody (guitar + piano).
//! Results are cached on disk as 16-bit so each song is only separated once.
//! If anything fails the caller falls back to the DSP separator in stems.rs.

use crate::stems::Stems;
use crate::tracks::SR;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const MODEL_URL: &str = "https://huggingface.co/StemSplitio/htdemucs-6s-onnx/resolve/main/htdemucs_6s_fp16weights.onnx";
const MODEL_FILE: &str = "htdemucs_6s_fp16weights.onnx";
const MODEL_MIN_BYTES: u64 = 100_000_000;
const MSR: u32 = 44100;
const SEG: usize = 343_980;
const CACHE_VER: u32 = 1;
const CACHE_CAP: u64 = 3 * 1024 * 1024 * 1024;

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("VibeCraft")
}

fn model_path() -> PathBuf {
    data_dir().join("models").join(MODEL_FILE)
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModelState {
    Missing,
    Downloading(f32),
    Ready,
    Failed(String),
}

pub static MODEL: Mutex<ModelState> = Mutex::new(ModelState::Missing);

fn set_state(s: ModelState) {
    *MODEL.lock().unwrap() = s;
}

pub fn model_state() -> ModelState {
    let s = MODEL.lock().unwrap().clone();
    if s == ModelState::Missing && model_path().metadata().map(|m| m.len() >= MODEL_MIN_BYTES).unwrap_or(false) {
        set_state(ModelState::Ready);
        return ModelState::Ready;
    }
    s
}

/// Downloads the model in the background if it is not there yet.
pub fn ensure_model() {
    if model_state() != ModelState::Missing {
        return;
    }
    set_state(ModelState::Downloading(0.0));
    std::thread::spawn(|| match download_model() {
        Ok(()) => set_state(ModelState::Ready),
        Err(e) => {
            eprintln!("sep: model download failed: {}", e);
            set_state(ModelState::Failed(e));
        }
    });
}

fn download_model() -> Result<(), String> {
    let path = model_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let part = path.with_extension("onnx.part");
    let resp = ureq::get(MODEL_URL).set("User-Agent", "vibe-craft").call().map_err(|e| e.to_string())?;
    let total: f32 = resp.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(136e6);
    let mut rd = resp.into_reader();
    let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = 0u64;
    loop {
        let n = rd.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got += n as u64;
        set_state(ModelState::Downloading((got as f32 / total).min(1.0)));
    }
    drop(f);
    if got < MODEL_MIN_BYTES {
        let _ = std::fs::remove_file(&part);
        return Err(format!("download too small ({} bytes)", got));
    }
    std::fs::rename(&part, &path).map_err(|e| e.to_string())
}

static SESSION: Mutex<Option<ort::session::Session>> = Mutex::new(None);

fn with_session<T>(f: impl FnOnce(&mut ort::session::Session) -> Result<T, String>) -> Result<T, String> {
    let mut g = SESSION.lock().unwrap();
    if g.is_none() {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        // Leave cores free for audio and drawing.
        let use_threads = (threads.saturating_sub(2)).max(1);
        let s = ort::session::Session::builder()
            .map_err(|e| e.to_string())?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
            .map_err(|e| e.to_string())?
            .with_intra_threads(use_threads)
            .map_err(|e| e.to_string())?
            .commit_from_file(model_path())
            .map_err(|e| e.to_string())?;
        *g = Some(s);
    }
    f(g.as_mut().unwrap())
}

/// Stereo decode at the file's own rate, capped at 9 minutes like the main decoder.
fn decode_stereo(path: &Path) -> Result<(Vec<f32>, Vec<f32>, u32), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).map_err(|e| e.to_string())?;
    let mut format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or("no audio track")?;
    let params = track.codec_params.clone();
    let id = track.id;
    let sr = params.sample_rate.ok_or("unknown sample rate")?;
    let mut dec = symphonia::default::get_codecs().make(&params, &DecoderOptions::default()).map_err(|e| e.to_string())?;
    let (mut l, mut r) = (Vec::new(), Vec::new());
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    let mut cap = 0usize;
    let limit = sr as usize * 60 * 9;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != id {
            continue;
        }
        match dec.decode(&packet) {
            Ok(buf) => {
                let ch = buf.spec().channels.count().max(1);
                if sbuf.is_none() || buf.capacity() > cap {
                    cap = buf.capacity();
                    sbuf = Some(SampleBuffer::<f32>::new(cap as u64, *buf.spec()));
                }
                let sb = sbuf.as_mut().unwrap();
                sb.copy_interleaved_ref(buf);
                for fr in sb.samples().chunks(ch) {
                    l.push(fr[0]);
                    r.push(if ch > 1 { fr[1] } else { fr[0] });
                }
                if l.len() > limit {
                    break;
                }
            }
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }
    Ok((l, r, sr))
}

/// Windowed-sinc resampler (good enough for analysis-grade audio, no aliasing on 2:1).
fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return x.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = (x.len() as f64 / ratio).round() as usize;
    let cutoff = (1.0 / ratio).min(1.0);
    let taps = 16isize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let c = i as f64 * ratio;
        let ci = c.floor() as isize;
        let mut acc = 0.0f64;
        let mut ws = 0.0f64;
        for k in (ci - taps * ratio.max(1.0) as isize)..=(ci + taps * ratio.max(1.0) as isize) {
            if k < 0 || k as usize >= x.len() {
                continue;
            }
            let t = (k as f64 - c) * cutoff;
            let sinc = if t.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * t).sin() / (std::f64::consts::PI * t) };
            let span = taps as f64 * ratio.max(1.0);
            let wv = 0.5 + 0.5 * (std::f64::consts::PI * (k as f64 - c) / span).cos();
            let w = sinc * wv;
            acc += x[k as usize] as f64 * w;
            ws += w;
        }
        out.push(if ws.abs() > 1e-9 { (acc / ws) as f32 } else { 0.0 });
    }
    out
}

/// Runs the model over a stereo 44.1 kHz song. Returns 6 stems x 2 channels.
fn run_model(l: &[f32], r: &[f32], progress: &AtomicU32) -> Result<Vec<[Vec<f32>; 2]>, String> {
    let total = l.len();
    let overlap = SEG / 4;
    let stride = SEG - overlap;
    let chunks = ((total + stride - 1) / stride).max(1);
    let mut win = vec![1.0f32; SEG];
    for i in 0..overlap {
        let f = i as f32 / (overlap - 1) as f32;
        win[i] = f;
        win[SEG - 1 - i] = f;
    }
    let mut out: Vec<[Vec<f32>; 2]> = (0..6).map(|_| [vec![0f32; total], vec![0f32; total]]).collect();
    let mut wsum = vec![0f32; total];
    let mut input = vec![0f32; 2 * SEG];
    for c in 0..chunks {
        let start = c * stride;
        let end = (start + SEG).min(total);
        let clen = end - start;
        input.iter_mut().for_each(|v| *v = 0.0);
        input[..clen].copy_from_slice(&l[start..end]);
        input[SEG..SEG + clen].copy_from_slice(&r[start..end]);
        let stems: Vec<f32> = with_session(|s| {
            let t = ort::value::Tensor::from_array(([1usize, 2, SEG], input.clone().into_boxed_slice())).map_err(|e| e.to_string())?;
            let outs = s.run(ort::inputs!["mix" => t]).map_err(|e| e.to_string())?;
            let (_, d) = outs["stems"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
            Ok(d.to_vec())
        })?;
        if stems.len() < 6 * 2 * SEG {
            return Err("unexpected model output size".into());
        }
        for s in 0..6 {
            for ch in 0..2 {
                let base = (s * 2 + ch) * SEG;
                let o = &mut out[s][ch];
                for i in 0..clen {
                    o[start + i] += stems[base + i] * win[i];
                }
            }
        }
        for i in 0..clen {
            wsum[start + i] += win[i];
        }
        progress.store(((c + 1) * 97 / chunks) as u32, Ordering::Relaxed);
    }
    for s in out.iter_mut() {
        for ch in s.iter_mut() {
            for (v, w) in ch.iter_mut().zip(&wsum) {
                *v /= w.max(1e-8);
            }
        }
    }
    Ok(out)
}

// ---- disk cache -----------------------------------------------------------

fn cache_key(path: &Path, n: usize) -> Option<String> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in path.to_string_lossy().bytes().chain(md.len().to_le_bytes()).chain(mtime.to_le_bytes()).chain((n as u64).to_le_bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    Some(format!("{:016x}", h))
}

fn cache_dir() -> PathBuf {
    data_dir().join("stems")
}

fn cache_load(key: &str, n: usize) -> Option<Stems> {
    let bytes = std::fs::read(cache_dir().join(format!("{}.vcs", key))).ok()?;
    if bytes.len() != 8 + crate::stems::NL * n * 2 || u32::from_le_bytes(bytes[0..4].try_into().ok()?) != CACHE_VER || u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize != n {
        return None;
    }
    let mut data: Vec<Arc<Vec<f32>>> = Vec::new();
    for k in 0..crate::stems::NL {
        let off = 8 + k * n * 2;
        let v: Vec<f32> = bytes[off..off + n * 2].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32767.0).collect();
        data.push(Arc::new(v));
    }
    Some(Stems { data: data.try_into().ok()?, ai: true })
}

fn cache_save(key: &str, s: &Stems, n: usize) {
    let dir = cache_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let mut bytes = Vec::with_capacity(8 + crate::stems::NL * n * 2);
    bytes.extend_from_slice(&CACHE_VER.to_le_bytes());
    bytes.extend_from_slice(&(n as u32).to_le_bytes());
    for k in 0..crate::stems::NL {
        for i in 0..n {
            let v = s.data[k].get(i).copied().unwrap_or(0.0);
            bytes.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
    }
    let _ = std::fs::write(dir.join(format!("{}.vcs", key)), bytes);
    // Keep the cache under its size cap, oldest first.
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd.filter_map(|e| e.ok()).filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        }).collect();
        files.sort();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        for (_, len, p) in files {
            if total <= CACHE_CAP {
                break;
            }
            let _ = std::fs::remove_file(p);
            total -= len;
        }
    }
}

/// Separate a song file into the app's layers, aligned to its `n`-sample 22 kHz mono pcm.
/// `mix` is that pcm; anything the model leaves out goes into Music so the layers
/// always add back up to the original.
pub fn separate_file(path: &Path, mix: &[f32], progress: &AtomicU32) -> Result<Stems, String> {
    let n = mix.len();
    let key = cache_key(path, n);
    if let Some(k) = &key {
        if let Some(s) = cache_load(k, n) {
            progress.store(100, Ordering::Relaxed);
            return Ok(s);
        }
    }
    if model_state() != ModelState::Ready {
        return Err("model not ready".into());
    }
    let (l, r, sr) = decode_stereo(path)?;
    let (l, r) = (resample(&l, sr, MSR), resample(&r, sr, MSR));
    let st = run_model(&l, &r, progress)?;
    // Model order: drums, bass, other, vocals, guitar, piano.
    let mono = |i: usize| -> Vec<f32> {
        let m: Vec<f32> = st[i][0].iter().zip(&st[i][1]).map(|(a, b)| 0.5 * (a + b)).collect();
        let mut d = resample(&m, MSR, SR);
        d.resize(n, 0.0);
        d
    };
    let drums = mono(0);
    let bass = mono(1);
    let other = mono(2);
    let vocals = mono(3);
    let guitar = mono(4);
    let piano = mono(5);
    let melody: Vec<f32> = guitar.iter().zip(&piano).map(|(a, b)| a + b).collect();
    let music: Vec<f32> = (0..n).map(|i| other[i] + (mix[i] - (drums[i] + bass[i] + other[i] + vocals[i] + melody[i]))).collect();
    let s = Stems { data: [Arc::new(vocals), Arc::new(music), Arc::new(bass), Arc::new(drums), Arc::new(melody)], ai: true };
    if let Some(k) = &key {
        cache_save(k, &s, n);
    }
    progress.store(100, Ordering::Relaxed);
    Ok(s)
}

fn write_wav(path: &Path, x: &[f32], sr: u32) {
    let mut b = Vec::with_capacity(44 + x.len() * 2);
    let data_len = (x.len() * 2) as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&sr.to_le_bytes());
    b.extend_from_slice(&(sr * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for v in x {
        b.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    let _ = std::fs::write(path, b);
}

/// `--sep-test <song> [out dir]`: downloads the model if needed, separates one song,
/// writes each layer as a WAV and prints timing and layer energies.
pub fn self_test(song: &Path, out: &Path) -> Result<(), String> {
    ensure_model();
    loop {
        match model_state() {
            ModelState::Ready => break,
            ModelState::Failed(e) => return Err(e),
            ModelState::Downloading(f) => eprint!("\rdownloading model {:.0}%   ", f * 100.0),
            ModelState::Missing => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    eprintln!();
    let (l, r, sr) = decode_stereo(song)?;
    let m: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
    let mix = resample(&m, sr, SR);
    // Skip the cache so the timing is real.
    let t0 = std::time::Instant::now();
    let prog = AtomicU32::new(0);
    let (l2, r2) = (resample(&l, sr, MSR), resample(&r, sr, MSR));
    let st = run_model(&l2, &r2, &prog)?;
    drop(st);
    let secs = t0.elapsed().as_secs_f32();
    let s = separate_file(song, &mix, &prog)?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    write_wav(&out.join("mix.wav"), &mix, SR);
    let e = |x: &[f32]| (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
    let em = e(&mix);
    for (k, name) in crate::stems::NAMES.iter().enumerate() {
        write_wav(&out.join(format!("{}.wav", name.to_lowercase())), &s.data[k], SR);
        println!("{:<7} {:5.1}% of mix level", name, 100.0 * e(&s.data[k]) / em);
    }
    let song_secs = mix.len() as f32 / SR as f32;
    println!("song {:.0} s, model time {:.1} s (x{:.2} real time)", song_secs, secs, secs / song_secs);
    Ok(())
}
