//! Word timing from the vocal layer (Whisper base, ONNX) and "shared word" transitions:
//! the next song is entered at a word that matches (or rhymes with) the last word the
//! outgoing singer finishes on.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const BASE: &str = "https://huggingface.co/onnx-community/whisper-base/resolve/main/";
const FILES: [(&str, &str, u64); 3] = [("onnx/encoder_model_quantized.onnx", "encoder.onnx", 20_000_000), ("onnx/decoder_model_merged_quantized.onnx", "decoder.onnx", 40_000_000), ("vocab.json", "vocab.json", 500_000)];
const LAYERS: usize = 6;
const HEADS: usize = 8;
const HDIM: usize = 64;
const SOT: i64 = 50258;
const EOT: i64 = 50257;
const TRANSCRIBE: i64 = 50359;
const TS0: i64 = 50364;
const LANG0: i64 = 50259;
const NLANG: i64 = 99;
/// Only this much vocal audio from the start point is transcribed (songs are mixed early).
const MAX_SECS: usize = 100;
const CHUNK: usize = 480_000; // 30 s at 16 kHz

#[derive(Clone, Debug)]
pub struct Word {
    /// Seconds from the start of the song.
    pub t0: f32,
    pub t1: f32,
    /// Lower-case letters and digits only.
    pub text: String,
}

pub type Words = Arc<Vec<Word>>;

pub static WORDS: Mutex<Vec<(usize, Words)>> = Mutex::new(Vec::new());
/// Bumped whenever a song's words arrive (the app re-plans the upcoming mix).
pub static VERSION: AtomicU32 = AtomicU32::new(0);
static BUSY: Mutex<Vec<usize>> = Mutex::new(Vec::new());

pub fn get(id: usize) -> Option<Words> {
    WORDS.lock().unwrap().iter().find(|w| w.0 == id).map(|w| w.1.clone())
}

fn dir() -> PathBuf {
    crate::sep::data_dir().join("models").join("whisper-base")
}

fn ready() -> bool {
    FILES.iter().all(|f| dir().join(f.1).metadata().map(|m| m.len() >= f.2).unwrap_or(false))
}

fn ensure_files() -> Result<(), String> {
    if ready() {
        return Ok(());
    }
    let mut done = 0f32;
    for (remote, local, min) in FILES {
        let p = dir().join(local);
        if p.metadata().map(|m| m.len() >= min).unwrap_or(false) {
            continue;
        }
        crate::sep::download(&format!("{}{}", BASE, remote), &p, min, None, &mut done, 1e9)?;
    }
    Ok(())
}

// ------------------------------------------------------------------ mel spectrogram

fn hz_to_mel(f: f64) -> f64 {
    if f < 1000.0 {
        f / (200.0 / 3.0)
    } else {
        15.0 + (f / 1000.0).ln() / (6.4f64.ln() / 27.0)
    }
}
fn mel_to_hz(m: f64) -> f64 {
    if m < 15.0 {
        m * 200.0 / 3.0
    } else {
        1000.0 * ((m - 15.0) * (6.4f64.ln() / 27.0)).exp()
    }
}

fn mel_filters() -> Vec<f32> {
    // 80 bands x 201 bins, Slaney scale and norm (as librosa / Whisper).
    let (nm, nf) = (80usize, 201usize);
    let fb: Vec<f64> = (0..nf).map(|i| i as f64 * 16000.0 / 400.0).collect();
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(8000.0));
    let pts: Vec<f64> = (0..nm + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (nm + 1) as f64)).collect();
    let mut w = vec![0f32; nm * nf];
    for m in 0..nm {
        let (a, b, c) = (pts[m], pts[m + 1], pts[m + 2]);
        let norm = 2.0 / (c - a);
        for (i, f) in fb.iter().enumerate() {
            let up = (f - a) / (b - a);
            let dn = (c - f) / (c - b);
            w[m * nf + i] = (up.min(dn).max(0.0) * norm) as f32;
        }
    }
    w
}

/// Log-mel of one 30 s chunk (zero-padded): 80 x 3000, row-major.
fn log_mel(x: &[f32], filt: &[f32], tab: &(Vec<f32>, Vec<f32>, Vec<f32>)) -> Vec<f32> {
    let n = CHUNK;
    let mut p = vec![0f32; n];
    p[..x.len().min(n)].copy_from_slice(&x[..x.len().min(n)]);
    let refl = |i: isize| -> f32 {
        let l = n as isize;
        let mut j = i;
        if j < 0 {
            j = -j;
        }
        if j >= l {
            j = 2 * (l - 1) - j;
        }
        p[j.clamp(0, l - 1) as usize]
    };
    let frames = 3000usize;
    let mut mel = vec![0f32; 80 * frames];
    let (win, cs, sn) = (&tab.0, &tab.1, &tab.2);
    let mut buf = [0f32; 400];
    let mut pw = [0f32; 201];
    for t in 0..frames {
        let s = t as isize * 160 - 200;
        for k in 0..400 {
            buf[k] = refl(s + k as isize) * win[k];
        }
        for b in 0..201 {
            let (mut re, mut im) = (0f32, 0f32);
            let row = &cs[b * 400..b * 400 + 400];
            let rows = &sn[b * 400..b * 400 + 400];
            for k in 0..400 {
                re += buf[k] * row[k];
                im += buf[k] * rows[k];
            }
            pw[b] = re * re + im * im;
        }
        for m in 0..80 {
            let f = &filt[m * 201..m * 201 + 201];
            let mut a = 0f32;
            for b in 0..201 {
                a += f[b] * pw[b];
            }
            mel[m * frames + t] = a.max(1e-10).log10();
        }
    }
    let mx = mel.iter().cloned().fold(f32::MIN, f32::max);
    for v in mel.iter_mut() {
        *v = (v.max(mx - 8.0) + 4.0) / 4.0;
    }
    mel
}

fn dft_tables() -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let win: Vec<f32> = (0..400).map(|k| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * k as f32 / 400.0).cos()).collect();
    let mut cs = vec![0f32; 201 * 400];
    let mut sn = vec![0f32; 201 * 400];
    for b in 0..201 {
        for k in 0..400 {
            let a = 2.0 * std::f64::consts::PI * (b * k % 400) as f64 / 400.0;
            cs[b * 400 + k] = a.cos() as f32;
            sn[b * 400 + k] = -a.sin() as f32;
        }
    }
    (win, cs, sn)
}

// ------------------------------------------------------------------ tokenizer

fn byte_decoder() -> HashMap<char, u8> {
    let mut bs: Vec<u32> = (33..=126).chain(161..=172).chain(174..=255).collect();
    let mut cs = bs.clone();
    let mut n = 0;
    for b in 0..256u32 {
        if !bs.contains(&b) {
            bs.push(b);
            cs.push(256 + n);
            n += 1;
        }
    }
    bs.iter().zip(cs.iter()).map(|(b, c)| (char::from_u32(*c).unwrap(), *b as u8)).collect()
}

struct Vocab {
    bytes: HashMap<i64, Vec<u8>>,
}

fn load_vocab() -> Result<Vocab, String> {
    let txt = std::fs::read_to_string(dir().join("vocab.json")).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
    let bd = byte_decoder();
    let mut bytes = HashMap::new();
    for (k, id) in v.as_object().ok_or("vocab")? {
        let id = id.as_i64().ok_or("vocab id")?;
        let b: Vec<u8> = k.chars().filter_map(|c| bd.get(&c).copied()).collect();
        bytes.insert(id, b);
    }
    Ok(Vocab { bytes })
}

// ------------------------------------------------------------------ model

struct Whisper {
    enc: ort::session::Session,
    dec: ort::session::Session,
    vocab: Vocab,
    filt: Vec<f32>,
    tab: (Vec<f32>, Vec<f32>, Vec<f32>),
    suppress: Vec<usize>,
}

fn session(path: &Path) -> Result<ort::session::Session, String> {
    ort::session::Session::builder()
        .map_err(|e| e.to_string())?
        .with_intra_threads(2)
        .map_err(|e| e.to_string())?
        .with_config_entry("session.intra_op.allow_spinning", "0")
        .map_err(|e| e.to_string())?
        .commit_from_file(path)
        .map_err(|e| e.to_string())
}

fn load() -> Result<Whisper, String> {
    ensure_files()?;
    crate::sep::init_runtime()?;
    let suppress: Vec<usize> = vec![
        1, 2, 7, 8, 9, 10, 14, 25, 26, 27, 28, 29, 31, 58, 59, 60, 61, 62, 63, 90, 91, 92, 93, 359, 503, 522, 542, 873, 893, 902, 918, 922, 931, 1350, 1853, 1982, 2460, 2627, 3246, 3253, 3268, 3536, 3846, 3961, 4183, 4667, 6585, 6647, 7273, 9061, 9383, 10428, 10929, 11938, 12033, 12331, 12562, 13793, 14157, 14635,
        15265, 15618, 16553, 16604, 18362, 18956, 20075, 21675, 22520, 26130, 26161, 26435, 28279, 29464, 31650, 32302, 32470, 36865, 42863, 47425, 49870, 50254, 50358, 50360, 50361, 50362,
    ];
    Ok(Whisper { enc: session(&dir().join("encoder.onnx"))?, dec: session(&dir().join("decoder.onnx"))?, vocab: load_vocab()?, filt: mel_filters(), tab: dft_tables(), suppress })
}

type Inp = Vec<(std::borrow::Cow<'static, str>, ort::session::SessionInputValue<'static>)>;

impl Whisper {
    fn encode(&mut self, pcm: &[f32]) -> Result<Vec<f32>, String> {
        let mel = log_mel(pcm, &self.filt, &self.tab);
        let t = ort::value::Tensor::from_array(([1usize, 80, 3000], mel.into_boxed_slice())).map_err(|e| e.to_string())?;
        let out = self.enc.run(ort::inputs!["input_features" => t]).map_err(|e| e.to_string())?;
        let (_, d) = out[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        Ok(d.to_vec())
    }

    /// Logits of the last position for `tokens`, without a key/value cache (short sequences).
    fn logits(&mut self, enc: &[f32], tokens: &[i64]) -> Result<Vec<f32>, String> {
        let mut inp: Inp = Vec::new();
        let ids = ort::value::Tensor::from_array(([1usize, tokens.len()], tokens.to_vec().into_boxed_slice())).map_err(|e| e.to_string())?;
        inp.push(("input_ids".into(), ids.into()));
        let eh = ort::value::Tensor::from_array(([1usize, 1500, 512], enc.to_vec().into_boxed_slice())).map_err(|e| e.to_string())?;
        inp.push(("encoder_hidden_states".into(), eh.into()));
        let uc = ort::value::Tensor::from_array(([1usize], vec![false].into_boxed_slice())).map_err(|e| e.to_string())?;
        inp.push(("use_cache_branch".into(), uc.into()));
        for l in 0..LAYERS {
            for (who, kv) in [("decoder", "key"), ("decoder", "value"), ("encoder", "key"), ("encoder", "value")] {
                let e = ort::value::Tensor::from_array(([1usize, HEADS, 1, HDIM], vec![0f32; HEADS * HDIM].into_boxed_slice())).map_err(|e| e.to_string())?;
                inp.push((format!("past_key_values.{}.{}.{}", l, who, kv).into(), e.into()));
            }
        }
        let out = self.dec.run(inp).map_err(|e| e.to_string())?;
        let (shape, d) = out["logits"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let v = shape[2] as usize;
        let n = shape[1] as usize;
        Ok(d[(n - 1) * v..n * v].to_vec())
    }

    fn detect_language(&mut self, enc: &[f32]) -> Result<i64, String> {
        let l = self.logits(enc, &[SOT])?;
        let mut best = (LANG0, f32::MIN);
        for id in LANG0..LANG0 + NLANG {
            if l[id as usize] > best.1 {
                best = (id, l[id as usize]);
            }
        }
        Ok(best.0)
    }

    /// Greedy decoding with timestamps; returns (start s, end s, text) segments inside the chunk.
    fn decode(&mut self, enc: &[f32], lang: i64) -> Result<Vec<(f32, f32, String)>, String> {
        let mut toks: Vec<i64> = vec![SOT, lang, TRANSCRIBE];
        let prompt = toks.len();
        for _ in 0..200 {
            let mut l = self.logits(enc, &toks)?;
            for &s in &self.suppress {
                l[s] = f32::NEG_INFINITY;
            }
            l[SOT as usize] = f32::NEG_INFINITY;
            l[50363] = f32::NEG_INFINITY;
            if toks.len() == prompt {
                l[220] = f32::NEG_INFINITY;
                l[EOT as usize] = f32::NEG_INFINITY;
            }
            let gen = &toks[prompt..];
            let last_ts = gen.last().map(|t| *t >= TS0).unwrap_or(false);
            let pen_ts = gen.len() >= 2 && gen[gen.len() - 2] >= TS0;
            if last_ts {
                if pen_ts {
                    // a finished pair: text or end comes next
                    for t in TS0 as usize..l.len() {
                        l[t] = f32::NEG_INFINITY;
                    }
                } else {
                    // a start stamp: a text token or the end stamp follows; never end-of-text here
                    l[EOT as usize] = f32::NEG_INFINITY;
                }
            } else if !gen.is_empty() {
                // text is running: the segment ends with a stamp, or the transcript ends
            }
            // timestamps never go backwards
            if let Some(&lt) = gen.iter().rev().find(|t| **t >= TS0) {
                let from = if last_ts && !pen_ts { lt } else { lt };
                for t in TS0..from.min(l.len() as i64) {
                    l[t as usize] = f32::NEG_INFINITY;
                }
            }
            if gen.is_empty() {
                for t in 0..TS0 as usize {
                    l[t] = f32::NEG_INFINITY;
                }
                for t in (TS0 + 50) as usize..l.len() {
                    l[t] = f32::NEG_INFINITY;
                }
            }
            // if the stamps together are likelier than any text token, say a stamp
            let mx = l.iter().cloned().fold(f32::MIN, f32::max);
            let lse = |r: std::ops::Range<usize>| -> f32 { mx + l[r].iter().map(|x| (x - mx).exp()).sum::<f32>().max(1e-30).ln() };
            let ts_lp = lse(TS0 as usize..l.len());
            let txt_max = l[..TS0 as usize].iter().cloned().fold(f32::MIN, f32::max);
            if !gen.is_empty() && ts_lp > txt_max {
                for t in 0..TS0 as usize {
                    l[t] = f32::NEG_INFINITY;
                }
            }
            let (mut bi, mut bv) = (0usize, f32::MIN);
            for (i, v) in l.iter().enumerate() {
                if *v > bv {
                    bv = *v;
                    bi = i;
                }
            }
            if bi as i64 == EOT {
                break;
            }
            toks.push(bi as i64);
        }
        let mut segs = Vec::new();
        let mut cur: Option<f32> = None;
        let mut bytes: Vec<u8> = Vec::new();
        for &t in &toks[prompt..] {
            if t >= TS0 {
                let s = (t - TS0) as f32 * 0.02;
                match cur {
                    None => cur = Some(s),
                    Some(a) => {
                        segs.push((a, s, String::from_utf8_lossy(&bytes).to_string()));
                        bytes.clear();
                        cur = None;
                    }
                }
            } else if t < EOT {
                if let Some(b) = self.vocab.bytes.get(&t) {
                    bytes.extend_from_slice(b);
                }
            }
        }
        if let Some(a) = cur {
            if !bytes.is_empty() {
                segs.push((a, (a + 3.0).min(30.0), String::from_utf8_lossy(&bytes).to_string()));
            }
        }
        Ok(segs)
    }
}

fn norm(w: &str) -> String {
    w.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Words of a song from its vocal layer (`vocal` mono at 22.05 kHz), starting at sample `from`.
pub fn transcribe_vocal(vocal: &[f32], from: usize) -> Result<Vec<Word>, String> {
    let mut w = load()?;
    let sr = crate::tracks::SR;
    let x = crate::sep::resample(vocal, sr, 16000);
    let start = (from as f64 / sr as f64 * 16000.0) as usize;
    let mut words = Vec::new();
    let mut lang: Option<i64> = None;
    let mut pos = start.min(x.len());
    let limit = (start + MAX_SECS * 16000).min(x.len());
    while pos < limit {
        let end = (pos + CHUNK).min(x.len());
        let chunk = &x[pos..end];
        if rms(chunk) < 0.004 {
            pos = end;
            continue;
        }
        let enc = w.encode(chunk)?;
        let lg = match lang {
            Some(l) => l,
            None => {
                let l = w.detect_language(&enc)?;
                lang = Some(l);
                l
            }
        };
        let t_chunk = pos as f32 / 16000.0;
        for (a, b, text) in w.decode(&enc, lg)? {
            let parts: Vec<&str> = text.split_whitespace().collect();
            let total: usize = parts.iter().map(|p| p.chars().count().max(1)).sum();
            if total == 0 {
                continue;
            }
            let mut acc = 0usize;
            for p in parts {
                let n = p.chars().count().max(1);
                let t0 = a + (b - a) * acc as f32 / total as f32;
                acc += n;
                let t1 = a + (b - a) * acc as f32 / total as f32;
                let text = norm(p);
                if !text.is_empty() {
                    words.push(Word { t0: t_chunk + t0, t1: t_chunk + t1, text });
                }
            }
        }
        pos = end;
    }
    Ok(words)
}

// ------------------------------------------------------------------ cache + background job

fn cache_path(key: &str) -> PathBuf {
    crate::sep::data_dir().join("words").join(format!("{}.txt", key))
}

fn save(key: &str, w: &[Word]) {
    let p = cache_path(key);
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    let s: String = w.iter().map(|x| format!("{:.2}\t{:.2}\t{}\n", x.t0, x.t1, x.text)).collect();
    let _ = std::fs::write(p, s);
}

fn load_cached(key: &str) -> Option<Vec<Word>> {
    let s = std::fs::read_to_string(cache_path(key)).ok()?;
    Some(
        s.lines()
            .filter_map(|l| {
                let mut it = l.split('\t');
                Some(Word { t0: it.next()?.parse().ok()?, t1: it.next()?.parse().ok()?, text: it.next()?.to_string() })
            })
            .collect(),
    )
}

/// Starts finding the words of track `id` in the background (once per song; cached on disk).
pub fn request(id: usize, path: Option<PathBuf>, n: usize, vocal: Arc<Vec<f32>>, from: usize) {
    if get(id).is_some() || std::env::var("VC_NO_WORDS").is_ok() {
        return;
    }
    {
        let mut b = BUSY.lock().unwrap();
        if b.contains(&id) {
            return;
        }
        b.push(id);
    }
    std::thread::spawn(move || {
        let key = path.as_ref().and_then(|p| crate::sep::cache_key(p, n));
        let t0 = std::time::Instant::now();
        let words = match key.as_ref().and_then(|k| load_cached(k)) {
            Some(w) => Some(w),
            None => match transcribe_vocal(&vocal, from) {
                Ok(w) => {
                    if let Some(k) = &key {
                        save(k, &w);
                    }
                    Some(w)
                }
                Err(e) => {
                    eprintln!("words: {}", e);
                    None
                }
            },
        };
        if let Some(w) = words {
            eprintln!("[{:7.1}s] words: song {} has {} words ({:.0} s)", crate::sep::uptime(), id, w.len(), t0.elapsed().as_secs_f32());
            let mut g = WORDS.lock().unwrap();
            g.retain(|x| x.0 != id);
            g.push((id, Arc::new(w)));
            while g.len() > 12 {
                g.remove(0);
            }
            VERSION.fetch_add(1, Ordering::SeqCst);
        }
        BUSY.lock().unwrap().retain(|x| *x != id);
    });
}

// ------------------------------------------------------------------ matching

/// 2 = same word, 1 = rhyme (same ending), 0 = no match.
pub fn match_score(a: &str, b: &str) -> u8 {
    if a == b && a.chars().count() >= 3 {
        return 2;
    }
    let (ca, cb): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let k = if ca.iter().any(|c| !c.is_ascii()) { 2 } else { 3 };
    if ca.len() >= k + 1 && cb.len() >= k + 1 && ca[ca.len() - k..] == cb[cb.len() - k..] && a != b {
        return 1;
    }
    0
}

pub struct Shared {
    pub in_pos: f64,
    pub word_out: String,
    pub word_in: String,
    pub exact: bool,
}

/// Finds an entry bar in the next song so that a word matching the end of the outgoing singer's
/// phrase lands where that phrase ends. Positions are samples (22.05 kHz); `bar_*` in samples.
/// `blend_beats` is the length of the blend; `lo_in`/`hi_in` limit where the entry may be.
#[allow(clippy::too_many_arguments)]
pub fn find_shared_word(wa: &[Word], out_pos: f64, beat_a: f64, wb: &[Word], origin_b: f64, bar_b: f64, lo_in: f64, hi_in: f64, blend_beats: f64) -> Option<Shared> {
    let sr = crate::tracks::SR as f64;
    let blend = blend_beats * beat_a;
    let beat_b = bar_b / 4.0;
    let mut best: Option<(f32, Shared)> = None;
    for (i, a) in wa.iter().enumerate() {
        let ta = a.t1 as f64 * sr;
        if ta < out_pos + 0.3 * blend || ta > out_pos + 0.9 * blend {
            continue;
        }
        // last word of a phrase
        let gap = wa.get(i + 1).map(|n| (n.t0 - a.t1) as f64).unwrap_or(9.0);
        if gap < 0.35 {
            continue;
        }
        let off_beats = (ta - out_pos) / beat_a;
        for b in wb {
            let m = match_score(&a.text, &b.text);
            if m == 0 {
                continue;
            }
            let tb = b.t1 as f64 * sr;
            let want = tb - off_beats * beat_b;
            let k = ((want - origin_b) / bar_b).round();
            let pos = origin_b + k * bar_b;
            if pos < lo_in || pos > hi_in {
                continue;
            }
            let err = ((tb - pos) / beat_b - off_beats).abs();
            if err > 1.0 {
                continue;
            }
            let score = m as f32 * 2.0 - err as f32;
            if best.as_ref().map(|x| score > x.0).unwrap_or(true) {
                best = Some((score, Shared { in_pos: pos, word_out: a.text.clone(), word_in: b.text.clone(), exact: m == 2 }));
            }
        }
    }
    best.map(|b| b.1)
}

/// `--words-test <song>`: transcribes a song's separated vocals and prints the words.
pub fn self_test(song: &Path) -> Result<(), String> {
    crate::sep::ensure_model();
    while matches!(crate::sep::model_state(), crate::sep::ModelState::Downloading(_) | crate::sep::ModelState::Missing) {
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    let (l, r, sr) = crate::sep::decode_stereo(song)?;
    let mono: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
    let mix = crate::sep::resample(&mono, sr, crate::tracks::SR);
    let prog = AtomicU32::new(0);
    let s = crate::sep::separate_file(song, &mix, &prog, &std::sync::atomic::AtomicBool::new(false))?;
    let t0 = std::time::Instant::now();
    let w = transcribe_vocal(&s.data[0], 0)?;
    println!("{} words in {:.0} s", w.len(), t0.elapsed().as_secs_f32());
    for x in &w {
        println!("{:7.2} {:7.2}  {}", x.t0, x.t1, x.text);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(t: f32, s: &str) -> Word {
        Word { t0: t, t1: t + 0.4, text: s.to_string() }
    }
    #[test]
    fn rhyme_and_same_word() {
        assert_eq!(match_score("night", "night"), 2);
        assert_eq!(match_score("night", "light"), 1);
        assert_eq!(match_score("night", "day"), 0);
    }
    #[test]
    fn finds_entry_bar_for_matching_word() {
        let sr = crate::tracks::SR as f64;
        // 120 bpm: beat 0.5 s. Outgoing phrase ends on "night" 8 beats (4 s) into a 16-beat blend.
        let wa = vec![w(103.0, "feel"), w(103.5, "the"), w(103.6, "night")];
        // ends at 104.0 s; the blend starts at 100 s -> 8 beats in
        let wb = vec![w(30.0, "hello"), w(37.6, "light"), w(39.0, "again")];
        let s = find_shared_word(&wa, 100.0 * sr, 0.5 * sr, &wb, 0.0, 2.0 * sr, 8.0 * sr, 60.0 * sr, 16.0).expect("match");
        assert_eq!(s.word_in, "light");
        // light ends at 38.0 s = beat 76 -> entry 8 beats earlier = beat 68 = 34.0 s = bar 17
        assert!((s.in_pos - 34.0 * sr).abs() < 1.0);
    }
}
