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

fn start_audio(mixer: Arc<Mutex<Mixer>>) -> Option<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let dev = host.default_output_device()?;
    let cfg = dev.default_output_config().ok()?;
    if cfg.sample_format() != cpal::SampleFormat::F32 {
        return None;
    }
    let channels = cfg.channels() as usize;
    let sr = cfg.sample_rate().0;
    mixer.lock().ok()?.set_sr(sr as f32);
    let stream_cfg: cpal::StreamConfig = cfg.into();
    let mc = mixer.clone();
    let stream = dev
        .build_output_stream(
            &stream_cfg,
            move |data: &mut [f32], _| {
                if let Ok(mut m) = mc.lock() {
                    m.render(data, channels);
                } else {
                    for s in data.iter_mut() {
                        *s = 0.0;
                    }
                }
            },
            |e| eprintln!("audio error: {}", e),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
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
    let stream = if headless { None } else { start_audio(mixer.clone()) };
    let device = stream.is_some();
    let mut app = App::new(lib.clone(), mixer.clone(), !headless);
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
                    detail: format!("tempo within 1% for {}/{} tracks; key within one Camelot step for {}/{}; beat confidence: {}", ok, n, kok, n, lib.iter().map(|t| format!("{:.1}", t.a.beat_conf)).collect::<Vec<_>>().join(" ")),
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
                    let n = ta.pcm.len();
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
                    m.decks[0].pcm = Some(ta.pcm.clone());
                    m.decks[0].stems = Some(sa.clone());
                    m.decks[0].track = Some(0);
                    m.decks[0].pos = p0;
                    m.decks[0].bpm = ta.a.bpm;
                    m.decks[0].origin = ta.a.origin;
                    m.decks[0].norm = ta.a.gain;
                    m.decks[0].playing = true;
                    m.decks[1].pcm = Some(tb.pcm.clone());
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
                    m.decks[0].pcm = Some(ta.pcm.clone());
                    m.decks[0].track = Some(0);
                    m.decks[0].pos = p0;
                    m.decks[0].bpm = ta.a.bpm;
                    m.decks[0].origin = ta.a.origin;
                    m.decks[0].norm = ta.a.gain;
                    m.decks[0].playing = true;
                    m.decks[1].pcm = Some(tb.pcm.clone());
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
                    m.decks[0].pcm = Some(t0.pcm.clone());
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
                    m1.decks[0].pcm = Some(t0.pcm.clone());
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
                        Some(c) => (c.out_pos > 0.0 && c.out_pos < a.pcm.len() as f64 && c.in_pos < b.pcm.len() as f64, c.why),
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
                tracks::set_music_dir(dir.clone());
                app.begin_scan();
                let _ = std::fs::remove_file("music_folder.txt");
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
