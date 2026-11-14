mod app;
mod dj;
mod engine;
mod screens;
mod stemctl;
mod stems;
mod tracks;
mod ui;

use app::{App, Armed};
use engine::{Mixer, Style};
use macroquad::prelude::*;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tracks::{build_demo, SR};

fn conf() -> Conf {
    Conf { window_title: "Smart AI DJ".to_string(), window_width: 1280, window_height: 760, high_dpi: false, ..Default::default() }
}

fn list_outputs() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    host.output_devices().map(|it| it.filter_map(|d| d.name().ok()).collect()).unwrap_or_default()
}

static AUDIO_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static DJ_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

static LAST_AUDIO_ERR: Mutex<String> = Mutex::new(String::new());

fn audio_err(s: String) {
    eprintln!("audio: {}", s);
    if let Ok(mut e) = LAST_AUDIO_ERR.lock() {
        *e = s;
    }
}

fn build_stream<T>(dev: &cpal::Device, cfg: &cpal::StreamConfig, mixer: Arc<Mutex<Mixer>>, dj: bool) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let channels = cfg.channels as usize;
    let sr = cfg.sample_rate.0;
    let mut tmp: Vec<f32> = Vec::new();
    dev.build_output_stream(
        cfg,
        move |data: &mut [T], _| {
            tmp.resize(data.len(), 0.0);
            if let Ok(mut m) = mixer.lock() {
                if dj { m.render_cue(&mut tmp, channels, sr as f32); } else { m.render(&mut tmp, channels); }
            } else {
                tmp.iter_mut().for_each(|s| *s = 0.0);
            }
            for (o, s) in data.iter_mut().zip(tmp.iter()) {
                *o = T::from_sample(*s);
            }
        },
        move |e| {
            eprintln!("audio error: {}", e);
            if dj { DJ_FAILED.store(true, std::sync::atomic::Ordering::Relaxed); } else { AUDIO_FAILED.store(true, std::sync::atomic::Ordering::Relaxed); }
        },
        None,
    )
    .map_err(|e| e.to_string())
}

/// Opens the named output device (None = system default). Returns the stream and device name.
fn start_audio(mixer: Arc<Mutex<Mixer>>, want: Option<&str>, dj: bool) -> Option<(cpal::Stream, String)> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let dev = match want {
        Some(n) => match host.output_devices().ok().and_then(|mut it| it.find(|d| d.name().ok().as_deref() == Some(n))) {
            Some(d) => d,
            None => {
                audio_err(format!("device '{}' not found", n));
                return None;
            }
        },
        None => match host.default_output_device() {
            Some(d) => d,
            None => {
                audio_err("no default output device".into());
                return None;
            }
        },
    };
    let dev_name = dev.name().unwrap_or_default();
    if dj { DJ_FAILED.store(false, std::sync::atomic::Ordering::Relaxed); } else { AUDIO_FAILED.store(false, std::sync::atomic::Ordering::Relaxed); }
    let cfg = match dev.default_output_config() {
        Ok(c) => c,
        Err(e) => {
            audio_err(format!("{}: {}", dev_name, e));
            return None;
        }
    };
    let fmt = cfg.sample_format();
    if !dj {
        mixer.lock().ok()?.set_sr(cfg.sample_rate().0 as f32);
    }
    let sc: cpal::StreamConfig = cfg.into();
    let r = match fmt {
        cpal::SampleFormat::F32 => build_stream::<f32>(&dev, &sc, mixer, dj),
        cpal::SampleFormat::I16 => build_stream::<i16>(&dev, &sc, mixer, dj),
        cpal::SampleFormat::U16 => build_stream::<u16>(&dev, &sc, mixer, dj),
        cpal::SampleFormat::I32 => build_stream::<i32>(&dev, &sc, mixer, dj),
        other => Err(format!("unsupported sample format {:?}", other)),
    };
    let stream = match r {
        Ok(s) => s,
        Err(e) => {
            audio_err(format!("{}: {}", dev_name, e));
            return None;
        }
    };
    if let Err(e) = stream.play() {
        audio_err(format!("{}: {}", dev_name, e));
        return None;
    }
    Some((stream, dev_name))
}

#[cfg(windows)]
fn send_to_back() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SetWindowPos};
    let title: Vec<u16> = "Smart AI DJ\0".encode_utf16().collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd != 0 {
            SetWindowPos(hwnd, 1, 0, 0, 0, 0, 0x0013);
        }
    }
}
#[cfg(not(windows))]
fn send_to_back() {}

struct Step {
    name: &'static str,
    passed: bool,
    detail: String,
}

fn write_results(path: &str, steps: &[Step]) {
    let mut s = String::from("{\"version\": 1, \"steps\": [");
    for (i, st) in steps.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&format!("{{\"step\": \"{}\", \"passed\": {}, \"detail\": \"{}\"}}", st.name, st.passed, st.detail.replace('"', "'").replace('\\', "/")));
    }
    s.push_str("]}");
    let _ = std::fs::write(path, s);
}

#[macroquad::main(conf)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned());
    let screenshot = arg("--screenshot");
    let selftest = arg("--self-test");
    let headless = screenshot.is_some() || selftest.is_some();
    clear_background(Color::new(0.04, 0.04, 0.06, 1.0));
    draw_text("Analysing the music library - detecting tempo, key and energy...", 40.0, 80.0, 30.0, WHITE);
    next_frame().await;
    if !headless {
        tracks::load_saved_music_dir();
        if tracks::CHOSEN_DIR.lock().unwrap().is_none() {
            if let Some(p) = tracks::find_default_music() {
                *tracks::CHOSEN_DIR.lock().unwrap() = Some(p);
            }
        }
    }
    if let Some(out) = arg("--automix-eval") {
        // End to end: the real AutoMix planner picks style and mix points for consecutive
        // songs, then each transition is rendered offline and measured.
        let dir = tracks::find_default_music().unwrap_or_else(|| std::path::PathBuf::from("music"));
        let prog = std::sync::Arc::new(Mutex::new(tracks::Progress::default()));
        tracks::start_scan(dir, prog.clone());
        while !prog.lock().unwrap().finished {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let mut tv = prog.lock().unwrap().result.take().unwrap_or_default();
        tracks::finalize(&mut tv);
        let lib = if args.iter().any(|a| a == "--demo") { Rc::new(build_demo()) } else { Rc::new(tv) };
        if args.iter().any(|a| a == "--classic") {
            app::CLASSIC_AUTOMIX.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let srf = SR as f64;
        let win = SR as usize / 2;
        let rmsw = |v: &[f32]| -> Vec<f32> { v.chunks(win).map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt()).collect() };
        let lowpass = |v: &[f32]| -> Vec<f32> {
            let a = 1.0 - (-2.0 * std::f32::consts::PI * 150.0 / SR as f32).exp();
            let mut lp = 0f32;
            v.iter().map(|&x| { lp += a * (x - lp); lp }).collect()
        };
        // onset envelope at 10 ms resolution
        let onsets = |v: &[f32]| -> Vec<f32> {
            let h = SR as usize / 100;
            let e: Vec<f32> = v.chunks(h).map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt()).collect();
            (0..e.len()).map(|i| if i == 0 { 0.0 } else { (e[i] - e[i - 1]).max(0.0) }).collect()
        };
        let med = |v: &[f32]| { let mut s = v.to_vec(); s.sort_by(|a, b| a.partial_cmp(b).unwrap()); s[s.len() / 2].max(1e-6) };
        // renders: mute 0 = mix, 1 = only outgoing, 2 = only incoming
        let run = |i: usize, j: usize, mute: u8| -> Option<(Vec<f32>, f64, f32, Style, String, f64, f64)> {
            let mixer = Arc::new(Mutex::new(Mixer::new()));
            mixer.lock().unwrap().set_sr(SR as f32);
            let mut app = App::new(lib.clone(), mixer.clone(), false);
            {
                let ta = &lib[i];
                let mut m = mixer.lock().unwrap();
                m.reset();
                m.set_sr(SR as f32);
                let d = &mut m.decks[0];
                d.pcm = Some(ta.audio());
                d.track = Some(i);
                d.pos = ta.samples() as f64 * 0.5;
                d.rate = 1.0;
                d.target_rate = 1.0;
                d.bpm = ta.a.bpm;
                d.origin = ta.a.origin;
                d.norm = ta.a.gain;
                d.playing = true;
                m.active = 0;
            }
            app.current = i;
            app.queue = vec![dj::QItem { track: j, pinned: true, manual: true, reason: String::new() }];
            app.plan(false);
            let armed = app.armed.as_ref()?;
            let (trigger, style, why) = (armed.trigger, armed.style, armed.why.clone());
            let mut m = mixer.lock().unwrap();
            let dur = m.pending.map(|p| p.dur_secs).unwrap_or(6.0);
            let start = (trigger - 4.0 * srf).max(0.0);
            m.decks[0].pos = start;
            if mute == 2 {
                m.decks[0].norm = 0.0;
            }
            if mute == 1 {
                m.decks[1].norm = 0.0;
            }
            let secs = 4.0 + dur as f64 + 6.0;
            let mut buf = vec![0f32; (secs * srf) as usize * 2];
            m.render(&mut buf, 2);
            let a_left = (lib[i].samples() as f64 - trigger) / srf;
            Some((buf.chunks(2).map(|f| f[0]).collect(), trigger, dur, style, why, a_left, (trigger - start) / srf))
        };
        let reference = |i: usize| -> f32 {
            let t = &lib[i];
            let mut m = Mixer::new();
            m.set_sr(SR as f32);
            m.decks[0].pcm = Some(t.audio());
            m.decks[0].pos = t.samples() as f64 * 0.5;
            m.decks[0].bpm = t.a.bpm;
            m.decks[0].norm = t.a.gain;
            m.decks[0].playing = true;
            let mut buf = vec![0f32; SR as usize * 6 * 2];
            m.render(&mut buf, 2);
            med(&rmsw(&buf.chunks(2).map(|f| f[0]).collect::<Vec<_>>()))
        };
        let mut text = String::new();
        let (mut n, mut sag_sum, mut bad_sag, mut bov_sum, mut beat_n, mut beat_ok, mut fades) = (0f32, 0f32, 0, 0f32, 0, 0, 0);
        let count = lib.len().min(40);
        for i in 0..count.saturating_sub(1) {
            let j = i + 1;
            let (Some(full), Some(a_only), Some(b_only)) = (run(i, j, 0), run(i, j, 1), run(i, j, 2)) else { continue };
            let (mix, _trig, dur, style, why, a_left, lead) = full;
            let r = (reference(i) * reference(j)).sqrt();
            // short-term loudness: power averaged over 3 s, like a loudness meter
            let rt0 = rmsw(&mix);
            let rt: Vec<f32> = (0..rt0.len()).map(|k| { let (a, b) = (k.saturating_sub(3), (k + 3).min(rt0.len())); (rt0[a..b].iter().map(|x| x * x).sum::<f32>() / (b - a) as f32).sqrt() }).collect();
            let k0 = (lead * 2.0) as usize;
            let k1 = ((lead + dur as f64 + 3.0) * 2.0) as usize;
            let seg = &rt[k0.min(rt.len() - 1)..k1.min(rt.len())];
            let sag = 20.0 * (seg.iter().cloned().fold(f32::MAX, f32::min) / r).log10();
            if args.iter().any(|a| a == "--dump") {
                text.push_str(&format!("  loudness every 0.5 s (dB vs typical, transition starts at {:.0} s): {}
", lead, rt.iter().map(|v| format!("{:.0}", 20.0 * (v / r).max(1e-6).log10())).collect::<Vec<_>>().join(" ")));
            }
            let (la, lb) = (rmsw(&lowpass(&a_only.0)), rmsw(&lowpass(&b_only.0)));
            let mut bov = 0f32;
            let ke = ((lead + dur as f64) * 2.0) as usize;
            for k in k0..ke.min(la.len()) {
                bov += la[k].min(lb[k]) / la[k].max(lb[k]).max(1e-6);
            }
            bov /= (ke - k0).max(1) as f32;
            // beat alignment over the overlap: best cross-correlation lag between the two decks
            let mut offs = String::from("-");
            if style != Style::Fade && style != Style::Plain {
                let (oa, ob) = (onsets(&a_only.0), onsets(&b_only.0));
                let (s0, s1) = ((lead * 100.0) as usize, ((lead + dur as f64) * 100.0) as usize);
                let s1 = s1.min(oa.len()).min(ob.len());
                if s1 > s0 + 100 {
                    let mut best = (f32::MIN, 0i32);
                    for lag in -25i32..=25 {
                        let mut s = 0f32;
                        for k in s0..s1 {
                            let k2 = k as i32 + lag;
                            if k2 >= 0 && (k2 as usize) < ob.len() {
                                s += oa[k] * ob[k2 as usize];
                            }
                        }
                        if s > best.0 {
                            best = (s, lag);
                        }
                    }
                    beat_n += 1;
                    if best.1.abs() <= 2 {
                        beat_ok += 1;
                    }
                    offs = format!("{} ms", best.1 * 10);
                }
            } else {
                fades += 1;
            }
            n += 1.0;
            sag_sum += sag;
            if sag < -6.0 {
                bad_sag += 1;
            }
            bov_sum += bov;
            text.push_str(&format!("{} -> {} | {} | {:.1}s overlap, {:.0}s before A ends | deepest sag {:+.1} dB | bass overlap {:.0}% | beat offset {} | {}\n", lib[i].title, lib[j].title, style.name(), dur, a_left, sag, bov * 100.0, offs, why));
        }
        text.push_str(&format!("SUMMARY {} transitions: {} plain fades; average deepest sag {:+.1} dB; sags worse than -6 dB: {}; bass overlap {:.0}%; beat-matched with beats within 20 ms: {}/{}\n", n, fades, sag_sum / n, bad_sag, 100.0 * bov_sum / n, beat_ok, beat_n));
        let _ = std::fs::write(out, text);
        std::process::exit(0);
    }
    if let Some(out) = arg("--mix-eval") {
        // Renders real transitions between songs with no shared beat and measures them.
        // Each song's own contribution is rendered separately, so we can see how much their
        // bass overlaps (0% = clean swap, 100% = two basslines at full strength together).
        let dir = tracks::find_default_music().unwrap_or_else(|| std::path::PathBuf::from("music"));
        let prog = std::sync::Arc::new(Mutex::new(tracks::Progress::default()));
        tracks::start_scan(dir, prog.clone());
        while !prog.lock().unwrap().finished {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let tv = prog.lock().unwrap().result.take().unwrap_or_default();
        let weak: Vec<&tracks::Track> = tv.iter().filter(|t| t.a.beat_conf < 1.5 || t.a.bpm < 1.0).collect();
        let styles = [Style::Plain, Style::Fade];
        let mut text = String::new();
        let mut tot = [[0f32; 4]; 2];
        let mut ctrl = [0f32; 3];
        let mut n = 0f32;
        let win = SR as usize / 4;
        let rmsw = |v: &[f32]| -> Vec<f32> { v.chunks(win).map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt()).collect() };
        let lowpass = |v: &[f32]| -> Vec<f32> {
            let a = 1.0 - (-2.0 * std::f32::consts::PI * 150.0 / SR as f32).exp();
            let mut lp = 0f32;
            v.iter().map(|&x| { lp += a * (x - lp); lp }).collect()
        };
        let med = |v: &[f32]| { let mut s = v.to_vec(); s.sort_by(|a, b| a.partial_cmp(b).unwrap()); s[s.len() / 2].max(1e-6) };
        for pair in weak.chunks(2).filter(|p| p.len() == 2).take(14) {
            let (ta, tb) = (pair[0], pair[1]);
            n += 1.0;
            // mute: 0 = both audible, 1 = only A, 2 = only B, 3 = control (A alone, no transition)
            let render = |style: Style, mute: u8| -> Vec<f32> {
                let mut m = Mixer::new();
                m.set_sr(SR as f32);
                let sr = SR as f64;
                let p0 = ta.samples() as f64 * 0.4;
                m.decks[0].pcm = Some(ta.audio());
                m.decks[0].track = Some(0);
                m.decks[0].pos = p0;
                m.decks[0].bpm = ta.a.bpm;
                m.decks[0].origin = ta.a.origin;
                m.decks[0].norm = if mute == 2 { 0.0 } else { ta.a.gain };
                m.decks[0].playing = true;
                m.decks[1].pcm = Some(tb.audio());
                m.decks[1].track = Some(1);
                m.decks[1].bpm = tb.a.bpm;
                m.decks[1].origin = tb.a.origin;
                m.decks[1].norm = if mute == 1 { 0.0 } else { tb.a.gain };
                if mute != 3 {
                    m.pending = Some(engine::Pending { deck_in: 1, trigger: p0 + 5.0 * sr, start_pos: tb.samples() as f64 * 0.2, in_rate: 1.0, dur_secs: 6.0, style, out_bpm_eff: ta.a.bpm });
                }
                let mut buf = vec![0f32; SR as usize * 17 * 2];
                m.render(&mut buf, 2);
                buf.chunks(2).map(|f| f[0]).collect()
            };
            // windows are 0.25 s: A alone 1..5 s, transition 5..11 s (windows 20..44), B alone 12..17 s
            let c = render(Style::Fade, 3);
            let (ct, cl) = (rmsw(&c), rmsw(&lowpass(&c)));
            let (c_ref_t, c_ref_l) = (((med(&ct[4..20]) * med(&ct[48..68])).sqrt()), med(&cl[4..20]).max(med(&cl[48..68])));
            ctrl[0] += 20.0 * (cl[20..44].iter().cloned().fold(0.0, f32::max) / c_ref_l).log10();
            ctrl[1] += 20.0 * (ct[20..44].iter().cloned().fold(f32::MAX, f32::min) / c_ref_t).log10();
            ctrl[2] += 20.0 * (ct[20..44].iter().cloned().fold(0.0, f32::max) / c_ref_t).log10();
            for (si, &style) in styles.iter().enumerate() {
                let full = render(style, 0);
                let (a_only, b_only) = (render(style, 1), render(style, 2));
                let (ft, fl) = (rmsw(&full), rmsw(&lowpass(&full)));
                let (la, lb) = (rmsw(&lowpass(&a_only)), rmsw(&lowpass(&b_only)));
                let (ma, mb) = (rmsw(&a_only), rmsw(&b_only));
                // bass overlap: how much of the louder bassline is joined by the other at the same time
                let mut ov = 0f32;
                let mut ovm = 0f32;
                for k in 20..44 {
                    ov += la[k].min(lb[k]) / la[k].max(lb[k]).max(1e-6);
                    ovm += ma[k].min(mb[k]) / ma[k].max(mb[k]).max(1e-6);
                }
                ov /= 24.0;
                ovm /= 24.0;
                let ref_t = (med(&ft[4..20]) * med(&ft[48..68])).sqrt();
                let _ = fl;
                let dip = 20.0 * (ft[20..44].iter().cloned().fold(f32::MAX, f32::min) / ref_t).log10();
                let bump = 20.0 * (ft[20..44].iter().cloned().fold(0.0, f32::max) / ref_t).log10();
                tot[si][0] += ov;
                tot[si][1] += dip;
                tot[si][2] += bump;
                tot[si][3] += ovm;
                text.push_str(&format!("{} -> {} | {:?}: bass overlap {:.0}%, full-band overlap {:.0}%, dip {:+.1} dB, bump {:+.1} dB\n", ta.title, tb.title, style, ov * 100.0, ovm * 100.0, dip, bump));
            }
        }
        for (si, name) in ["Plain (old)", "Fade (new)"].iter().enumerate() {
            text.push_str(&format!("AVERAGE {}: bass overlap {:.0}%, full-band overlap {:.0}%, dip {:+.1} dB (control {:+.1}), bump {:+.1} dB (control {:+.1}) over {} pairs\n", name, 100.0 * tot[si][0] / n, 100.0 * tot[si][3] / n, tot[si][1] / n, ctrl[1] / n, tot[si][2] / n, ctrl[2] / n, n));
        }
        let _ = std::fs::write(out, text);
        std::process::exit(0);
    }
    if let Some(out) = arg("--beat-eval") {
        // Ground-truth-free check: tempo found on the first half must match the second half.
        let dir = tracks::find_default_music().unwrap_or_else(|| std::path::PathBuf::from("music"));
        let prog = std::sync::Arc::new(Mutex::new(tracks::Progress::default()));
        tracks::start_scan(dir, prog.clone());
        while !prog.lock().unwrap().finished {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let tv = prog.lock().unwrap().result.take().unwrap_or_default();
        let agree = |a: f32, b: f32| {
            let r = a / b;
            (r > 0.985 && r < 1.015) as u8
        };
        let (mut old_ok, mut new_ok, mut n) = (0, 0, 0);
        let mut rows = String::new();
        for t in tv.iter() {
            let pcm = t.audio();
            let h = pcm.len() / 2;
            let (a0, b0) = (tracks::detect_beats_with(&pcm[..h], false).0, tracks::detect_beats_with(&pcm[h..], false).0);
            let (a1, b1) = (tracks::detect_beats_with(&pcm[..h], true).0, tracks::detect_beats_with(&pcm[h..], true).0);
            n += 1;
            old_ok += agree(a0, b0) as i32;
            new_ok += agree(a1, b1) as i32;
            rows.push_str(&format!("{} | old {:.1}/{:.1} | new {:.1}/{:.1}\n", t.title, a0, b0, a1, b1));
        }
        rows.push_str(&format!("SUMMARY halves agree: old {}/{}  new {}/{}\n", old_ok, n, new_ok, n));
        let _ = std::fs::write(out, rows);
        std::process::exit(0);
    }
    if let Some(out) = arg("--scan-report") {
        let dir = tracks::find_default_music().unwrap_or_else(|| std::path::PathBuf::from("music"));
        let prog = std::sync::Arc::new(Mutex::new(tracks::Progress::default()));
        let t0 = std::time::Instant::now();
        tracks::start_scan(dir.clone(), prog.clone());
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if prog.lock().unwrap().finished {
                break;
            }
        }
        let mut p = prog.lock().unwrap();
        let tracks_v = p.result.take().unwrap_or_default();
        let mut s = format!("{{\"folder\": \"{}\", \"seconds\": {:.1}, \"found\": {}, \"loaded\": {}, \"failed\": {}, \"tracks\": [", dir.to_string_lossy().replace('\\', "/"), t0.elapsed().as_secs_f32(), p.found, tracks_v.len(), p.failed.len());
        for (i, t) in tracks_v.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&format!("{{\"title\": \"{}\", \"artist\": \"{}\", \"dur\": {:.0}, \"bpm\": {:.1}, \"conf\": {:.1}, \"key\": \"{}\", \"first_bar\": {}, \"last_bar\": {}, \"bars\": {}}}", t.title.replace('"', "'"), t.artist.replace('"', "'"), t.dur, t.a.bpm, t.a.beat_conf, tracks::camelot_str(t.a.key, t.a.minor), t.a.first_bar, t.a.last_bar, t.a.bars.len()));
        }
        s.push_str("], \"failures\": [");
        for (i, f) in p.failed.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&format!("\"{}\"", f.replace('"', "'").replace('\\', "/")));
        }
        s.push_str("]}");
        let _ = std::fs::write(out, s);
        std::process::exit(0);
    }
    ui::init_fonts();
    let lib = Rc::new(build_demo());
    let mixer = Arc::new(Mutex::new(Mixer::new()));
    let mut stream: Option<cpal::Stream> = None;
    let mut want_dev: Option<String> = None;
    let mut dj_stream: Option<cpal::Stream> = None;
    let mut out_name = String::new();
    if !headless {
        if let Some((s, n)) = start_audio(mixer.clone(), None, false) {
            stream = Some(s);
            out_name = n;
        }
    }

    let mut app = App::new(lib.clone(), mixer.clone(), !headless);
    if !headless {
        app.out_devices = list_outputs();
        app.out_name = out_name;
    }
    let mut dev_poll = 0.0f32;
    if !headless && tracks::CHOSEN_DIR.lock().unwrap().is_some() {
        app.begin_scan();
    }
    if let Some(t) = arg("--tab").and_then(|s| s.parse::<usize>().ok()) {
        app.tab = t.min(3);
    }
    let mut frame: u32 = 0;
    let mut steps: Vec<Step> = Vec::new();
    let mut pos0 = 0.0f64;
    let mut cur0 = app.current;
    let mut armed_at_skip: Option<Armed> = None;
    let mut sec_info: Option<(usize, (f64, f64), u32)> = None;
    loop {
        frame += 1;
        let dt = get_frame_time().min(0.1);
        if !headless {
            dev_poll += dt;
            let failed = AUDIO_FAILED.load(std::sync::atomic::Ordering::Relaxed);
            if dev_poll > 3.0 {
                dev_poll = 0.0;
                app.out_devices = list_outputs();
            }
            // switch request from the UI, or reopen after the device vanished/errored
            let req = app.out_request.take();
            if req.is_some() || failed || (stream.is_none() && dev_poll == 0.0) {
                if let Some(r) = req {
                    want_dev = r;
                }
                stream = None; // drop the old stream first
                let mut res = start_audio(mixer.clone(), want_dev.as_deref(), false);
                if res.is_none() && want_dev.is_some() {
                    want_dev = None;
                    res = start_audio(mixer.clone(), None, false);
                }
                match res {
                    Some((s, n)) => {
                        stream = Some(s);
                        app.out_name = n.clone();
                        if !app.dj_name.is_empty() && app.dj_name == n {
                            dj_stream = None;
                            app.dj_name.clear();
                            mixer.lock().unwrap().dj_on = false;
                            app.say("DJ output turned off: it was the same device as the audience");
                        } else {
                            app.say(&format!("Audience output: {}", n));
                        }
                    }
                    None => {
                        app.out_name.clear();
                    }
                }
            }
        }
        if !headless {
            // DJ (headphone) output: "" = off, "*" = system default, otherwise a device name
            let dj_failed = DJ_FAILED.load(std::sync::atomic::Ordering::Relaxed);
            if dj_failed {
                dj_stream = None;
                app.dj_name.clear();
                mixer.lock().unwrap().dj_on = false;
                app.say("DJ output lost - turned off");
                DJ_FAILED.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            if let Some(r) = app.dj_request.take() {
                dj_stream = None;
                app.dj_name.clear();
                mixer.lock().unwrap().dj_on = false;
                if !r.is_empty() {
                    let want = if r == "*" { None } else { Some(r.as_str()) };
                    match start_audio(mixer.clone(), want, true) {
                        Some((s, n)) => {
                            if n == app.out_name {
                                drop(s);
                                app.say("DJ output must be a different device from the audience output");
                            } else {
                                dj_stream = Some(s);
                                app.dj_name = n.clone();
                                let mut m = mixer.lock().unwrap();
                                m.dj_on = true;
                                m.cue_restart();
                                drop(m);
                                app.say(&format!("DJ output: {}", n));
                            }
                        }
                        None => {
                            let e = LAST_AUDIO_ERR.lock().map(|e| e.clone()).unwrap_or_default();
                            app.say(&format!("DJ output failed: {}", e));
                        }
                    }
                }
            }
        }
        let _keep_dj = &dj_stream;
        let device = stream.is_some();
        let audio = if headless {
            Some(0.25)
        } else if device {
            None
        } else {
            Some(dt)
        };
        app.update(dt, audio);
        app.draw();
        if headless && frame < 6 {
            send_to_back();
        }
        if let Some(path) = &screenshot {
            if frame >= 120 {
                if let Some(parent) = std::path::Path::new(path).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                get_screen_data().export_png(path);
                std::process::exit(0);
            }
        }
        if let Some(path) = &selftest {
            if frame % 12 == 0 && frame < 300 {
                app.tab = (app.tab + 1) % 4;
                app.scroll = 0.0;
            }
            if frame == 2 {
                app.tab = 0;
                let (mut ok, mut kok, mut n) = (0, 0, 0);
                for t in lib.iter() {
                    if let (Some(tb), Some((r, mi))) = (t.true_bpm, t.true_key) {
                        n += 1;
                        if (t.a.bpm - tb).abs() / tb < 0.01 {
                            ok += 1;
                        }
                        let (c1, _) = tracks::camelot(t.a.key, t.a.minor);
                        let (c2, _) = tracks::camelot(r, mi);
                        let d = (c1 - c2).abs();
                        if d == 0 || d == 1 || d == 11 {
                            kok += 1;
                        }
                    }
                }
                steps.push(Step {
                    name: "analysis",
                    passed: n > 0 && ok * 10 >= n * 7 && kok * 10 >= n * 6,
                    detail: format!("tempo within 1% for {}/{} tracks (missed: {}); key within one Camelot step for {}/{}; beat confidence: {}", ok, n, lib.iter().filter(|t| t.true_bpm.map_or(false, |tb| (t.a.bpm - tb).abs() / tb >= 0.01)).map(|t| format!("{} true {:.1} got {:.1}", t.title, t.true_bpm.unwrap(), t.a.bpm)).collect::<Vec<_>>().join("; "), kok, n, lib.iter().map(|t| format!("{:.1}", t.a.beat_conf)).collect::<Vec<_>>().join(" ")),
                });
                steps.push(Step { name: "queue", passed: app.queue.len() >= 3, detail: format!("DJ filled the queue with {} tracks before playback", app.queue.len()) });
                {
                    // layer separation: the four layers must add back up to the song and each must carry something
                    let t0 = std::time::Instant::now();
                    let prog = std::sync::atomic::AtomicU32::new(0);
                    let (ta, tb) = (&lib[0], &lib[1]);
                    let sa = std::sync::Arc::new(stems::separate(&ta.pcm, &prog));
                    let sb = std::sync::Arc::new(stems::separate(&tb.pcm, &prog));
                    let secs = t0.elapsed().as_secs_f32();
                    let n = ta.samples();
                    let (mut err, mut tot) = (0f64, 0f64);
                    for i in (n / 4..n * 3 / 4).step_by(7) {
                        let sum: f32 = (0..4).map(|k| sa.data[k][i]).sum();
                        let d = (sum - ta.pcm[i]) as f64;
                        err += d * d;
                        tot += (ta.pcm[i] as f64).powi(2);
                    }
                    let rel = (err / tot.max(1e-12)).sqrt();
                    let raw_e: Vec<f64> = (0..4).map(|k| (n / 4..n * 3 / 4).step_by(7).map(|i| (sa.data[k][i] as f64).powi(2)).sum::<f64>()).collect();
                    let sum_e: f64 = raw_e.iter().sum::<f64>().max(1e-12);
                    let energy: Vec<f64> = raw_e.iter().map(|e| e / sum_e).collect();
                    let all = energy.iter().all(|e| *e > 0.005);
                    steps.push(Step {
                        name: "layers",
                        passed: rel < 0.02 && all,
                        detail: format!("two songs separated in {:.1} s; layers add back to the song within {:.2}%; energy share vocals {:.0}% music {:.0}% bass {:.0}% drums {:.0}%", secs, rel * 100.0, energy[0] * 100.0, energy[1] * 100.0, energy[2] * 100.0, energy[3] * 100.0),
                    });
                    // the same blend with layer-aware transitions
                    let mut m = Mixer::new();
                    m.set_sr(SR as f32);
                    let bar = 4.0 * SR as f64 * 60.0 / ta.a.bpm as f64;
                    let bar_b = 4.0 * SR as f64 * 60.0 / tb.a.bpm as f64;
                    let p0 = ta.a.origin + 8.0 * bar;
                    m.decks[0].pcm = Some(ta.audio());
                    m.decks[0].stems = Some(sa.clone());
                    m.decks[0].track = Some(0);
                    m.decks[0].pos = p0;
                    m.decks[0].bpm = ta.a.bpm;
                    m.decks[0].origin = ta.a.origin;
                    m.decks[0].norm = ta.a.gain;
                    m.decks[0].playing = true;
                    m.decks[1].pcm = Some(tb.audio());
                    m.decks[1].stems = Some(sb.clone());
                    m.decks[1].track = Some(1);
                    m.decks[1].bpm = tb.a.bpm;
                    m.decks[1].origin = tb.a.origin;
                    m.decks[1].norm = tb.a.gain;
                    let rate = ta.a.bpm as f64 / tb.a.bpm as f64;
                    let dur = 16.0 * 60.0 / ta.a.bpm;
                    m.pending = Some(engine::Pending { deck_in: 1, trigger: p0 + 2.0 * bar, start_pos: tb.a.origin + 8.0 * bar_b, in_rate: rate, dur_secs: dur, style: Style::Blend, out_bpm_eff: ta.a.bpm });
                    let mut buf = vec![0f32; SR as usize * 12 * 2];
                    m.render(&mut buf, 2);
                    let w = SR as usize / 2;
                    let mut rms: Vec<f32> = Vec::new();
                    let mut peak = 0f32;
                    for c in buf.chunks(w * 2) {
                        let s: f32 = c.chunks(2).map(|f| f[0] * f[0]).sum::<f32>() / w as f32;
                        rms.push(s.sqrt());
                    }
                    for v in &buf {
                        peak = peak.max(v.abs());
                    }
                    let body: Vec<f32> = rms[1..rms.len() - 1].to_vec();
                    let mut sorted = body.clone();
                    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    let med = sorted[sorted.len() / 2];
                    let lo = body.iter().cloned().fold(f32::MAX, f32::min);
                    let hi = body.iter().cloned().fold(0.0, f32::max);
                    steps.push(Step {
                        name: "layer_mix",
                        passed: m.completed >= 1 && lo > 0.55 * med && hi < 1.7 * med && peak < 0.98,
                        detail: format!("layer-aware blend finished: {}; loudest {:.1} dB / quietest {:.1} dB vs typical; peak {:.2}", m.completed >= 1, 20.0 * (hi / med).log10(), 20.0 * (lo / med).log10(), peak),
                    });
                }
                {
                    // loudness stays even through a beat-matched blend
                    let (ta, tb) = (&lib[0], &lib[1]);
                    let mut m = Mixer::new();
                    m.set_sr(SR as f32);
                    let bar = 4.0 * SR as f64 * 60.0 / ta.a.bpm as f64;
                    let bar_b = 4.0 * SR as f64 * 60.0 / tb.a.bpm as f64;
                    let p0 = ta.a.origin + 8.0 * bar;
                    m.decks[0].pcm = Some(ta.audio());
                    m.decks[0].track = Some(0);
                    m.decks[0].pos = p0;
                    m.decks[0].bpm = ta.a.bpm;
                    m.decks[0].origin = ta.a.origin;
                    m.decks[0].norm = ta.a.gain;
                    m.decks[0].playing = true;
                    m.decks[1].pcm = Some(tb.audio());
                    m.decks[1].track = Some(1);
                    m.decks[1].bpm = tb.a.bpm;
                    m.decks[1].origin = tb.a.origin;
                    m.decks[1].norm = tb.a.gain;
                    let rate = ta.a.bpm as f64 / tb.a.bpm as f64;
                    let dur = 16.0 * 60.0 / ta.a.bpm;
                    m.pending = Some(engine::Pending { deck_in: 1, trigger: p0 + 2.0 * bar, start_pos: tb.a.origin + 8.0 * bar_b, in_rate: rate, dur_secs: dur, style: Style::Blend, out_bpm_eff: ta.a.bpm });
                    let secs = 12usize;
                    let mut buf = vec![0f32; SR as usize * secs * 2];
                    m.render(&mut buf, 2);
                    let w = SR as usize / 2;
                    let mut rms: Vec<f32> = Vec::new();
                    for c in buf.chunks(w * 2) {
                        let s: f32 = c.chunks(2).map(|f| f[0] * f[0]).sum::<f32>() / w as f32;
                        rms.push(s.sqrt());
                    }
                    let body: Vec<f32> = rms[1..rms.len() - 1].to_vec();
                    let mut sorted = body.clone();
                    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    let med = sorted[sorted.len() / 2];
                    let lo = body.iter().cloned().fold(f32::MAX, f32::min);
                    let hi = body.iter().cloned().fold(0.0, f32::max);
                    steps.push(Step {
                        name: "mix_loudness",
                        passed: m.completed >= 1 && lo > 0.55 * med && hi < 1.7 * med,
                        detail: format!("blend finished: {}; loudest {:.1} dB / quietest {:.1} dB vs typical", m.completed >= 1, 20.0 * (hi / med).log10(), 20.0 * (lo / med).log10()),
                    });
                }
                {
                    // several transition points are found in a song
                    let n_out = dj::top_out_points(&lib[0], 0.0, 8).len();
                    let n_in = dj::top_in_points(&lib[0], 8).len();
                    steps.push(Step { name: "multiple_points", passed: n_out >= 2 && n_in >= 2, detail: format!("{} exit points and {} entry points found in one song", n_out, n_in) });
                }
                {
                    // time-stretch quality: tempo moves, pitch (key) stays
                    let t0 = &lib[0];
                    let mut m = Mixer::new();
                    m.set_sr(SR as f32);
                    m.decks[0].pcm = Some(t0.audio());
                    m.decks[0].track = Some(0);
                    m.decks[0].pos = t0.a.origin;
                    m.decks[0].rate = 1.06;
                    m.decks[0].target_rate = 1.06;
                    m.decks[0].bpm = t0.a.bpm;
                    m.decks[0].playing = true;
                    let mut buf = vec![0f32; SR as usize * 40 * 2];
                    m.render(&mut buf, 2);
                    let mono: Vec<f32> = buf.chunks(2).map(|c| c[0]).collect();
                    let (bpm, _, _, _) = tracks::detect_beats(&mono);
                    let (k, mi, _) = tracks::detect_key(&mono);
                    let want = t0.a.bpm * 1.06;
                    let mut m1 = Mixer::new();
                    m1.set_sr(SR as f32);
                    m1.decks[0].pcm = Some(t0.audio());
                    m1.decks[0].pos = t0.a.origin;
                    m1.decks[0].bpm = t0.a.bpm;
                    m1.decks[0].playing = true;
                    let mut buf1 = vec![0f32; SR as usize * 40 * 2];
                    m1.render(&mut buf1, 2);
                    let mono1: Vec<f32> = buf1.chunks(2).map(|c| c[0]).collect();
                    let (k1, mi1, _) = tracks::detect_key(&mono1);
                    let (c1, _) = tracks::camelot(k, mi);
                    let (c2, _) = tracks::camelot(k1, mi1);
                    let _ = (c1, c2);
                    let sine: Vec<f32> = (0..SR as usize * 20).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SR as f32).sin()).collect();
                    let mut m2 = Mixer::new();
                    m2.set_sr(SR as f32);
                    m2.decks[0].pcm = Some(std::sync::Arc::new(sine));
                    m2.decks[0].pos = 1000.0;
                    m2.decks[0].rate = 1.06;
                    m2.decks[0].target_rate = 1.06;
                    m2.decks[0].playing = true;
                    let mut buf2 = vec![0f32; SR as usize * 5 * 2];
                    m2.render(&mut buf2, 2);
                    let mono2: Vec<f32> = buf2.chunks(2).skip(SR as usize).map(|c| c[0]).collect();
                    let p_keep = app::goertzel(&mono2, 440.0, SR as f32);
                    let p_shift = app::goertzel(&mono2, 440.0 * 1.06, SR as f32);
                    let ok = (bpm - want).abs() / want < 0.015 && p_keep > 5.0 * p_shift;
                    steps.push(Step { name: "time_stretch", passed: ok, detail: format!("played at +6%: tempo {:.1} (wanted {:.1}), pitch kept: 440 Hz tone power {:.3} vs shifted {:.3}; key reads {} (unstretched {})", bpm, want, p_keep, p_shift, tracks::key_name(k, mi), tracks::key_name(k1, mi1)) });
                    let (a, b) = (&lib[0], &lib[1]);
                    let c = dj::choose_mix(a, b, 0.0, 1.0, 16, false, dj::MixOpts::default());
                    let (pass, det) = match c {
                        Some(c) => (c.out_pos > 0.0 && c.out_pos < a.samples() as f64 && c.in_pos < b.samples() as f64, c.why),
                        None => (false, "no mix points found".to_string()),
                    };
                    steps.push(Step { name: "automix_points", passed: pass, detail: det });
                }
                pos0 = app.snap.pos[app.snap.active];
                cur0 = app.current;
            }
            if frame == 40 {
                let a = app.snap.active;
                let moved = (app.snap.pos[a] - pos0) / SR as f64;
                let peak = mixer.lock().unwrap().peak_seen;
                steps.push(Step { name: "move", passed: moved > 6.0 && peak > 0.1, detail: format!("playback advanced {:.1} s, output peak {:.2}", moved, peak) });
                app.skip();
                armed_at_skip = app.armed.clone();
            }
            if frame == 90 {
                let (completed, jump) = {
                    let m = mixer.lock().unwrap();
                    (m.completed, m.max_jump)
                };
                steps.push(Step {
                    name: "act",
                    passed: completed >= 1 && app.current != cur0,
                    detail: format!("skip mixed into '{}' (transitions completed: {})", lib[app.current].title, completed),
                });
                let (pass, det) = match &armed_at_skip {
                    Some(ar) => {
                        let tin = &lib[ar.track];
                        let eff_in = tin.a.bpm as f64 * ar.in_rate;
                        let err = (eff_in - ar.out_eff as f64).abs() / ar.out_eff as f64;
                        if ar.matched && ar.style != Style::EchoOut {
                            (err < 0.005, format!("{:?}: incoming {:.2} vs outgoing {:.2} BPM", ar.style, eff_in, ar.out_eff))
                        } else {
                            (true, format!("{:?}: tempos not matchable, handled with echo transition", ar.style))
                        }
                    }
                    None => (false, "no transition plan".to_string()),
                };
                steps.push(Step { name: "beat_match", passed: pass, detail: det });
                steps.push(Step { name: "smooth_mix", passed: jump < 1.2, detail: format!("largest sample-to-sample jump {:.2}", jump) });
                {
                    let mut m = mixer.lock().unwrap();
                    let a = m.active;
                    let t = &lib[app.current];
                    let back = 14.0 * SR as f64 * m.decks[a].rate;
                    m.decks[a].pos = (t.a.outro_start - back).max(0.0);
                }
                app.dirty = true;
                cur0 = app.current;
            }
            if frame == 100 {
                let dir = std::path::PathBuf::from("shots/scan_test");
                let _ = std::fs::create_dir_all(&dir);
                let pcm = &lib[0].pcm;
                let n = (SR as usize * 30).min(pcm.len());
                let mut wav: Vec<u8> = Vec::new();
                wav.extend_from_slice(b"RIFF");
                wav.extend_from_slice(&((36 + n * 2) as u32).to_le_bytes());
                wav.extend_from_slice(b"WAVEfmt ");
                wav.extend_from_slice(&16u32.to_le_bytes());
                wav.extend_from_slice(&1u16.to_le_bytes());
                wav.extend_from_slice(&1u16.to_le_bytes());
                wav.extend_from_slice(&SR.to_le_bytes());
                wav.extend_from_slice(&(SR * 2).to_le_bytes());
                wav.extend_from_slice(&2u16.to_le_bytes());
                wav.extend_from_slice(&16u16.to_le_bytes());
                wav.extend_from_slice(b"data");
                wav.extend_from_slice(&((n * 2) as u32).to_le_bytes());
                for s in &pcm[..n] {
                    wav.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32000.0) as i16).to_le_bytes());
                }
                let _ = std::fs::write(dir.join("test song.wav"), wav);
                // keep the user's saved music folder: the test must not overwrite or delete it
                let saved = std::fs::read("music_folder.txt").ok();
                tracks::set_music_dir(dir.clone());
                app.begin_scan();
                match saved {
                    Some(bytes) => {
                        let _ = std::fs::write("music_folder.txt", bytes);
                    }
                    None => {
                        let _ = std::fs::remove_file("music_folder.txt");
                    }
                }
            }
            if frame == 320 {
                let n = app.lib.iter().filter(|t| t.user).count();
                steps.push(Step { name: "scan_folder", passed: n >= 1 && !app.scanning(), detail: format!("background scan loaded {} song(s) from a chosen folder; {}", n, app.scan_msg) });
            }
            if frame == 330 {
                let n = app.snap.completed;
                steps.push(Step { name: "auto_mix", passed: app.current != cur0 && n >= 2, detail: format!("Auto DJ mixed on its own into '{}' (transitions: {})", lib[app.current].title, n) });
                let need = ["title", "artist", "bpm", "key", "progress", "queue", "mood", "visualizer", "dj_cards", "transition", "explain", "controls", "library"];
                let missing: Vec<&str> = need.iter().cloned().filter(|t| !app.drawn.contains(t)).collect();
                steps.push(Step {
                    name: "hud",
                    passed: missing.is_empty(),
                    detail: if missing.is_empty() { "title, artist, BPM, key, progress, queue, mood, visualiser, DJ screen and library are drawn".to_string() } else { format!("missing: {}", missing.join(", ")) },
                });
                let a_pos = app.snap.pos[app.snap.active];
                let cur = app.current;
                let span = 24.0 * SR as f64 * app.snap.rate[app.snap.active];
                app.set_section(cur, a_pos, a_pos + span);
                sec_info = Some((cur, app.sections[cur].unwrap(), app.snap.completed));
            }
            if frame == 345 {
                if let Some((cur, (a, b), _)) = sec_info {
                    let bar = 4.0 * SR as f64 * 60.0 / lib[cur].a.bpm as f64;
                    let ok = match &app.armed {
                        Some(ar) => ar.trigger >= a && ar.trigger <= b + bar,
                        None => false,
                    };
                    let det = match &app.armed {
                        Some(ar) => format!("section {}-{}; mix starts at {}", dj::clock_str(a), dj::clock_str(b), dj::clock_str(ar.trigger)),
                        None => "no plan".to_string(),
                    };
                    steps.push(Step { name: "section_plan", passed: ok, detail: det });
                }
            }
            if frame == 460 {
                if let Some((cur, (_, _), c0)) = sec_info {
                    steps.push(Step { name: "section_play", passed: app.snap.completed > c0 && app.current != cur, detail: format!("only the chosen section played, then the DJ moved on (transitions {} -> {})", c0, app.snap.completed) });
                }
                write_results(path, &steps);
                std::process::exit(0);
            }
        }
        next_frame().await;
    }
}
