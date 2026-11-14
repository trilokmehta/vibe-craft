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

// ONNX Runtime with DirectML (MIT licence) is loaded at run time from
// %LOCALAPPDATA%\VibeCraft\runtime so the app stays a single .exe that the
// auto-updater can replace. DirectML itself comes with Windows 10/11.
const RUNTIME_VER: &str = "dml-1.22.1";
const RUNTIME_URL: &str = "https://github.com/trilokmehta/vibe-craft/releases/download/gpu-runtime-1.22.1/onnxruntime.dll";
const RUNTIME_SHA256: &str = "f722dd91a311b9fa4b7f2341d22c379a72fb7bc5b639df18b64821f23cdde14b";
const RUNTIME_BYTES: f32 = 16.5e6;
const MODEL_BYTES: f32 = 136.4e6;

fn runtime_path() -> PathBuf {
    data_dir().join("runtime").join(RUNTIME_VER).join("onnxruntime.dll")
}

fn runtime_ok() -> bool {
    runtime_path().metadata().map(|m| m.len() > 10_000_000).unwrap_or(false)
}

fn model_ok() -> bool {
    model_path().metadata().map(|m| m.len() >= MODEL_MIN_BYTES).unwrap_or(false)
}

pub fn model_state() -> ModelState {
    let s = MODEL.lock().unwrap().clone();
    if s == ModelState::Missing && model_ok() && runtime_ok() {
        set_state(ModelState::Ready);
        return ModelState::Ready;
    }
    s
}

/// Downloads the AI runtime and model in the background if they are not there yet.
pub fn ensure_model() {
    if model_state() != ModelState::Missing {
        return;
    }
    set_state(ModelState::Downloading(0.0));
    std::thread::spawn(|| {
        let total = (if runtime_ok() { 0.0 } else { RUNTIME_BYTES }) + if model_ok() { 0.0 } else { MODEL_BYTES };
        let mut done = 0f32;
        let r = (|| -> Result<(), String> {
            if !runtime_ok() {
                download(RUNTIME_URL, &runtime_path(), 10_000_000, Some(RUNTIME_SHA256), &mut done, total)?;
            }
            if !model_ok() {
                download(MODEL_URL, &model_path(), MODEL_MIN_BYTES, None, &mut done, total)?;
            }
            Ok(())
        })();
        match r {
            Ok(()) => set_state(ModelState::Ready),
            Err(e) => {
                eprintln!("sep: download failed: {}", e);
                set_state(ModelState::Failed(e));
            }
        }
    });
}

fn download(url: &str, path: &Path, min_bytes: u64, sha256: Option<&str>, done: &mut f32, total: f32) -> Result<(), String> {
    use sha2::Digest;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let part = path.with_extension("part");
    let resp = ureq::get(url).set("User-Agent", "vibe-craft").call().map_err(|e| e.to_string())?;
    let mut rd = resp.into_reader();
    let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = 0u64;
    let mut hasher = sha2::Sha256::new();
    loop {
        let n = rd.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hasher.update(&buf[..n]);
        got += n as u64;
        *done += n as f32;
        set_state(ModelState::Downloading((*done / total.max(1.0)).min(1.0)));
    }
    drop(f);
    if got < min_bytes {
        let _ = std::fs::remove_file(&part);
        return Err(format!("download too small ({} bytes)", got));
    }
    if let Some(want) = sha256 {
        let have: String = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect();
        if have != want {
            let _ = std::fs::remove_file(&part);
            return Err("download failed its integrity check".into());
        }
    }
    std::fs::rename(&part, path).map_err(|e| e.to_string())
}


static RUNTIME_INIT: std::sync::Once = std::sync::Once::new();
/// Where separation runs, for the UI: "" until the first song, then "GPU" or "CPU".
pub static DEVICE: Mutex<&'static str> = Mutex::new("");

pub fn device() -> &'static str {
    *DEVICE.lock().unwrap()
}

fn build_session_from(gpu: bool, path: &Path) -> Result<ort::session::Session, String> {
    use ort::session::builder::GraphOptimizationLevel;
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    // Leave cores free for audio and drawing; the GPU path needs few CPU threads.
    // Half the cores at most, and no busy-waiting, so playback never stutters.
    let use_threads = if gpu { 2 } else { (threads / 2).max(1) };
    let mut b = ort::session::Session::builder()
        .map_err(|e| e.to_string())?
        .with_optimization_level(match std::env::var("VC_OPT").as_deref() { Ok("0") => GraphOptimizationLevel::Disable, Ok("1") => GraphOptimizationLevel::Level1, Ok("2") => GraphOptimizationLevel::Level2, _ => GraphOptimizationLevel::Level3 })
        .map_err(|e| e.to_string())?
        .with_intra_threads(use_threads)
        .map_err(|e| e.to_string())?
        .with_config_entry("session.intra_op.allow_spinning", "0")
        .map_err(|e| e.to_string())?
        .with_config_entry("session.inter_op.allow_spinning", "0")
        .map_err(|e| e.to_string())?;
    if !gpu {
        // No memory arena: it keeps the peak allocation (over a GB) for the app's lifetime.
        b = b
            .with_execution_providers([ort::execution_providers::CPUExecutionProvider::default().with_arena_allocator(false).build()])
            .map_err(|e| e.to_string())?;
    }
    if gpu {
        // DirectML needs these two off. Graph fusion is also off: on this model it
        // needs more video memory than an 8 GB card has.
        b = b
            .with_config_entry("ep.dml.disable_graph_fusion", "1")
            .map_err(|e| e.to_string())?
            .with_memory_pattern(false)
            .map_err(|e| e.to_string())?
            .with_parallel_execution(false)
            .map_err(|e| e.to_string())?
            .with_execution_providers([ort::execution_providers::DirectMLExecutionProvider::default().build().error_on_failure(), ort::execution_providers::CPUExecutionProvider::default().with_arena_allocator(false).build()])
            .map_err(|e| e.to_string())?;
    }
    b.commit_from_file(path).map_err(|e| e.to_string())
}

enum Engine {
    /// Whole model in one session (CPU).
    Single(ort::session::Session),
    /// Part A on the GPU, part B (inverse STFT) on the CPU; see split.rs.
    Split { a: ort::session::Session, b: ort::session::Session, b_inputs: Vec<(String, i32)> },
}

fn part_paths() -> (PathBuf, PathBuf, PathBuf) {
    let d = data_dir().join("models");
    (d.join("htdemucs_6s.partA.onnx"), d.join("htdemucs_6s.partB.onnx"), d.join("htdemucs_6s.partB.inputs"))
}

fn dyn_type(v: &ort::value::DynValue) -> Option<i32> {
    use ort::tensor::TensorElementType as T;
    match v.dtype() {
        ort::value::ValueType::Tensor { ty, .. } => Some(match ty {
            T::Float32 => 1,
            T::Uint8 => 2,
            T::Int8 => 3,
            T::Int32 => 6,
            T::Int64 => 7,
            T::Bool => 9,
            T::Float16 => 10,
            T::Float64 => 11,
            _ => return None,
        }),
        _ => None,
    }
}

fn split_engine() -> Result<Engine, String> {
    let (pa, pb, pi) = part_paths();
    let a = if pa.exists() {
        build_session_from(true, &pa)?
    } else {
        eprintln!("sep: splitting the model for the GPU (one time)...");
        let m = crate::split::load(&model_path())?;
        let (ma, mb, _) = crate::split::split(&m)?;
        crate::split::save(&ma, &pa)?;
        crate::split::save(&mb, &pb.with_extension("untyped"))?;
        build_session_from(true, &pa)?
    };
    let b_inputs: Vec<(String, i32)> = match std::fs::read_to_string(&pi) {
        Ok(s) if pb.exists() => s.lines().filter_map(|l| l.split_once('\t')).map(|(n, t)| (n.to_string(), t.parse().unwrap_or(1))).collect(),
        _ => {
            // Run part A once to learn the types of the tensors it hands to part B.
            let mut a = a;
            let zero = ort::value::Tensor::from_array(([1usize, 2, SEG], vec![0f32; 2 * SEG].into_boxed_slice())).map_err(|e| e.to_string())?;
            let outs = a.run(ort::inputs!["mix" => zero]).map_err(|e| e.to_string())?;
            let mut types: Vec<(String, i32)> = Vec::new();
            for (name, v) in outs.iter() {
                types.push((name.to_string(), dyn_type(&v.view().into_dyn()).ok_or("unsupported tensor type between parts")?));
            }
            drop(outs);
            let untyped = pb.with_extension("untyped");
            let mut mb = crate::split::load(&untyped)?;
            // Part B also reads the original input when the graph does.
            let reads_mix = mb.graph.as_ref().map(|g| g.node.iter().any(|n| n.input.iter().any(|i| i == "mix"))).unwrap_or(false);
            if reads_mix {
                types.push(("mix".to_string(), 1));
            }
            crate::split::type_inputs(&mut mb, &types);
            crate::split::save(&mb, &pb)?;
            let _ = std::fs::remove_file(untyped);
            std::fs::write(&pi, types.iter().map(|(n, t)| format!("{}\t{}\n", n, t)).collect::<String>()).map_err(|e| e.to_string())?;
            let b = build_session_from(false, &pb)?;
            return Ok(Engine::Split { a, b, b_inputs: types });
        }
    };
    let b = build_session_from(false, &pb)?;
    Ok(Engine::Split { a, b, b_inputs })
}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

/// Runs the model on one 7.8 s stereo window; returns the flat [6, 2, SEG] stems.
fn infer(input: &[f32]) -> Result<Vec<f32>, String> {
    let mut g = ENGINE.lock().unwrap();
    if g.is_none() {
        let mut init_err = None;
        RUNTIME_INIT.call_once(|| {
            // Always load our own copy: Windows has an older onnxruntime.dll in System32.
            if let Err(e) = ort::init_from(runtime_path().to_string_lossy().to_string()).commit() {
                init_err = Some(e.to_string());
            }
        });
        if let Some(e) = init_err {
            return Err(format!("AI runtime failed to load: {}", e));
        }
        let want_gpu = std::env::var("VC_CPU").is_err();
        let e = match if want_gpu { split_engine() } else { Err("CPU forced".into()) } {
            Ok(e) => {
                *DEVICE.lock().unwrap() = "GPU";
                e
            }
            Err(err) => {
                if want_gpu {
                    eprintln!("sep: GPU unavailable ({}), using CPU", err);
                }
                *DEVICE.lock().unwrap() = "CPU";
                Engine::Single(build_session_from(false, &model_path())?)
            }
        };
        eprintln!("sep: separating on {}", device());
        *g = Some(e);
    }
    let mk = || ort::value::Tensor::from_array(([1usize, 2, SEG], input.to_vec().into_boxed_slice())).map_err(|e| e.to_string());
    let res = match g.as_mut().unwrap() {
        Engine::Single(s) => {
            let outs = s.run(ort::inputs!["mix" => mk()?]).map_err(|e| e.to_string())?;
            let (_, d) = outs["stems"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
            Ok(d.to_vec())
        }
        Engine::Split { a, b, b_inputs } => {
            let mut outs_a = a.run(ort::inputs!["mix" => mk()?]).map_err(|e| e.to_string())?;
            let mut feed: Vec<(String, ort::session::SessionInputValue)> = Vec::new();
            for (name, _) in b_inputs.iter() {
                if name == "mix" {
                    feed.push((name.clone(), mk()?.into_dyn().into()));
                } else {
                    let v = outs_a.remove(name.as_str()).ok_or_else(|| format!("part A did not produce {}", name))?;
                    feed.push((name.clone(), v.into()));
                }
            }
            let outs = b.run(feed).map_err(|e| e.to_string())?;
            let (_, d) = outs["stems"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
            Ok(d.to_vec())
        }
    };
    if res.is_err() && matches!(g.as_ref(), Some(Engine::Split { .. })) {
        // GPU failed mid-run (e.g. out of video memory): switch to CPU for good.
        eprintln!("sep: GPU run failed, switching to CPU");
        *DEVICE.lock().unwrap() = "CPU";
        *g = Some(Engine::Single(build_session_from(false, &model_path())?));
        drop(g);
        return infer(input);
    }
    res
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
    // Windowed-sinc resampler with the filter precomputed for 512 sub-sample
    // phases, so the inner loop is only multiply-adds (no sin/cos per tap).
    const PHASES: usize = 512;
    let ratio = from as f64 / to as f64;
    let n = (x.len() as f64 / ratio).round() as usize;
    let cutoff = (1.0 / ratio).min(1.0);
    let half = (16.0 * ratio.max(1.0)).ceil() as isize; // taps on each side
    let width = (2 * half + 1) as usize;
    let mut table = vec![0f32; PHASES * width];
    for p in 0..PHASES {
        let frac = p as f64 / PHASES as f64;
        let row = &mut table[p * width..(p + 1) * width];
        let mut sum = 0.0;
        for (j, w) in row.iter_mut().enumerate() {
            let d = (j as isize - half) as f64 - frac;
            let t = d * cutoff;
            let sinc = if t.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * t).sin() / (std::f64::consts::PI * t) };
            let win = 0.5 + 0.5 * (std::f64::consts::PI * d / (half as f64 + 1.0)).cos();
            *w = (sinc * win) as f32;
            sum += *w as f64;
        }
        // Normalise each phase so DC gain is exactly 1.
        row.iter_mut().for_each(|w| *w = (*w as f64 / sum) as f32);
    }
    let len = x.len() as isize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let c = i as f64 * ratio;
        let ci = c.floor() as isize;
        let p = (((c - ci as f64) * PHASES as f64) as usize).min(PHASES - 1);
        let row = &table[p * width..(p + 1) * width];
        let start = ci - half;
        let mut acc = 0f32;
        if start >= 0 && start + width as isize <= len {
            let seg = &x[start as usize..start as usize + width];
            for j in 0..width {
                acc += seg[j] * row[j];
            }
        } else {
            for j in 0..width {
                let k = start + j as isize;
                if k >= 0 && k < len {
                    acc += x[k as usize] * row[j];
                }
            }
        }
        out.push(acc);
    }
    out
}

/// Runs the model over a stereo 44.1 kHz song. Returns 6 stems x 2 channels.
/// Returns the 6 model stems as mono 44.1 kHz (stereo is folded down as each window
/// arrives, which halves the memory this needs).
fn run_model(l: &[f32], r: &[f32], progress: &AtomicU32, cancel: &std::sync::atomic::AtomicBool) -> Result<Vec<Vec<f32>>, String> {
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
    let mut out: Vec<Vec<f32>> = (0..6).map(|_| vec![0f32; total]).collect();
    let mut wsum = vec![0f32; total];
    let mut input = vec![0f32; 2 * SEG];
    for c in 0..chunks {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let start = c * stride;
        let end = (start + SEG).min(total);
        let clen = end - start;
        input.iter_mut().for_each(|v| *v = 0.0);
        input[..clen].copy_from_slice(&l[start..end]);
        input[SEG..SEG + clen].copy_from_slice(&r[start..end]);
        let stems: Vec<f32> = infer(&input)?;
        if stems.len() < 6 * 2 * SEG {
            return Err("unexpected model output size".into());
        }
        for s in 0..6 {
            let (bl, br) = (s * 2 * SEG, (s * 2 + 1) * SEG);
            let o = &mut out[s];
            for i in 0..clen {
                o[start + i] += 0.5 * (stems[bl + i] + stems[br + i]) * win[i];
            }
        }
        for i in 0..clen {
            wsum[start + i] += win[i];
        }
        progress.store(((c + 1) * 97 / chunks) as u32, Ordering::Relaxed);
        if std::env::var("VC_LOG").is_ok() {
            eprintln!("[{:7.1}s] sep: window {}/{} on {}", uptime(), c + 1, chunks, device());
        }
    }
    for s in out.iter_mut() {
        for (v, w) in s.iter_mut().zip(&wsum) {
            *v /= w.max(1e-8);
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
pub fn separate_file(path: &Path, mix: &[f32], progress: &AtomicU32, cancel: &std::sync::atomic::AtomicBool) -> Result<Stems, String> {
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
    let st = run_model(&l, &r, progress, cancel)?;
    drop((l, r));
    // Model order: drums, bass, other, vocals, guitar, piano.
    let mono = |i: usize| -> Vec<f32> {
        let mut d = resample(&st[i], MSR, SR);
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

pub fn write_wav(path: &Path, x: &[f32], sr: u32) {
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
    let never = std::sync::atomic::AtomicBool::new(false);
    let st = run_model(&l2, &r2, &prog, &never)?;
    let secs = t0.elapsed().as_secs_f32();
    let s = separate_file(song, &mix, &prog, &never)?;
    // Compare this run (on whatever device) with the cached result: model order
    // drums, bass, other, vocals; app layers vocals=0, bass=2, drums=3.
    for (name, mi, li) in [("vocals", 3usize, 0usize), ("bass", 1, 2), ("drums", 0, 3)] {
        let mut d = resample(&st[mi], MSR, SR);
        d.resize(mix.len(), 0.0);
        let c = &s.data[li];
        let (mut err, mut tot) = (0f64, 0f64);
        for i in (0..d.len()).step_by(3) {
            err += ((d[i] - c[i]) as f64).powi(2);
            tot += (c[i] as f64).powi(2);
        }
        println!("{:<7} this run vs cached result: {:.2}% difference", name, 100.0 * (err / tot.max(1e-12)).sqrt());
    }
    drop(st);
    println!("device: {}", device());
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

/// `--onnx-inspect`: prints the nodes at the end of the model graph (development aid).
pub fn inspect() {
    use protobuf::Message;
    let bytes = std::fs::read(model_path()).expect("model");
    let m = onnx_protobuf::ModelProto::parse_from_bytes(&bytes).expect("parse");
    let g = &m.graph;
    println!("nodes {} inits {} value_info {} inputs {:?} outputs {:?}", g.node.len(), g.initializer.len(), g.value_info.len(), g.input.iter().map(|i| &i.name).collect::<Vec<_>>(), g.output.iter().map(|i| &i.name).collect::<Vec<_>>());
    let first = g.node.iter().position(|n| n.name.starts_with("/real_istft")).unwrap_or(0);
    println!("first istft node at {}", first);
    for n in g.node.iter().skip(first.saturating_sub(3)) {
        println!("{:>5} {:<14} {:<40} in={:?} out={:?}", 0, n.op_type, n.name, n.input, n.output);
    }
}

/// Unloads the AI engine (several GB of committed memory with DirectML) when no
/// song is waiting to be separated. It is loaded again, in a few seconds, on demand.
pub fn release_engine() {
    if let Ok(mut g) = ENGINE.try_lock() {
        if g.is_some() {
            *g = None;
            eprintln!("[{:7.1}s] sep: engine unloaded (idle)", uptime());
        }
    }
}

/// Seconds since the app started (for log lines).
pub fn uptime() -> f32 {
    static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f32()
}

#[cfg(test)]
mod resample_tests {
    #[test]
    fn halves_rate_and_keeps_level() {
        let sr = 44100.0f32;
        let x: Vec<f32> = (0..44100).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sr).sin()).collect();
        let t = std::time::Instant::now();
        let y = super::resample(&x, 44100, 22050);
        assert!(t.elapsed().as_millis() < 200, "too slow: {:?}", t.elapsed());
        assert_eq!(y.len(), 22050);
        let rms = (y[2000..20000].iter().map(|v| v * v).sum::<f32>() / 18000.0).sqrt();
        assert!((rms - 0.7071).abs() < 0.01, "rms {}", rms);
        // 1 kHz at 22.05 kHz: 22.05 samples per cycle
        let zc = y[2000..20000].windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((zc as i32 - 816).abs() <= 2, "zero crossings {}", zc);
    }
}
