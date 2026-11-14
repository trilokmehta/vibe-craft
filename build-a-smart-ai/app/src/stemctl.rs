//! Background layer separation for the playing / next song and the layer controls.
use crate::app::*;
use crate::stems::Stems;
use crate::ui::*;
use crate::ui::{draw_text, measure_text};
use macroquad::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub struct StemJob {
    pub pct: AtomicU32,
    pub done: AtomicBool,
    pub result: Mutex<Option<Arc<Stems>>>,
    pub profile: Mutex<Option<Arc<crate::vibe::Profile>>>,
}

impl App {
    /// Separates layers only for the song that is playing and the one about to play.
    pub fn manage_stems(&mut self) {
        let mixer = self.mixer.clone();
        if !self.use_stems {
            if self.snap.stems.iter().any(|b| *b) {
                let mut m = mixer.lock().unwrap();
                for d in 0..2 {
                    m.decks[d].stems = None;
                }
            }
            return;
        }
        if let Some((id, job)) = self.stem_job.clone() {
            if job.done.load(Ordering::SeqCst) {
                if let Some(p) = job.profile.lock().unwrap().take() {
                    self.profiles.retain(|c| c.0 != id);
                    self.profiles.push((id, p));
                    // Re-plan the upcoming mix now that its layers are known.
                    if id == self.current || self.queue.first().map(|q| q.track) == Some(id) {
                        self.dirty = true;
                    }
                    while self.profiles.len() > 8 {
                        self.profiles.remove(0);
                    }
                }
                if let Some(s) = job.result.lock().unwrap().take() {
                    self.stem_cache.push((id, s));
                    while self.stem_cache.len() > 4 {
                        self.stem_cache.drain(0..1);
                    }
                }
                self.stem_job = None;
            }
        }
        {
            let mut m = mixer.lock().unwrap();
            for d in 0..2 {
                if m.decks[d].stems.is_none() {
                    if let Some(tid) = m.decks[d].track {
                        if let Some((_, s)) = self.stem_cache.iter().find(|c| c.0 == tid) {
                            m.decks[d].stems = Some(s.clone());
                        }
                    }
                }
            }
        }
        if self.stem_job.is_none() {
            let mut want = vec![self.current];
            if let Some(q) = self.queue.first() {
                want.push(q.track);
            }
            for tid in want {
                if self.stem_cache.iter().any(|c| c.0 == tid) {
                    continue;
                }
                let job = Arc::new(StemJob { pct: AtomicU32::new(0), done: AtomicBool::new(false), result: Mutex::new(None), profile: Mutex::new(None) });
                let path = self.lib[tid].path.clone();
                // Wait for the AI model while it downloads; songs without a file (demo) use DSP.
                if path.is_some() && matches!(crate::sep::model_state(), crate::sep::ModelState::Downloading(_)) {
                    break;
                }
                let (origin, bar, nbars) = {
                    let a = &self.lib[tid].a;
                    (a.origin, 4.0 * crate::tracks::SR as f64 * 60.0 / a.bpm.max(40.0) as f64, a.bars.len())
                };
                let pcm = self.lib[tid].audio();
                let j2 = job.clone();
                std::thread::spawn(move || {
                    let ai = path.as_ref().map(|p| crate::sep::separate_file(p, &pcm, &j2.pct));
                    let s = match ai {
                        Some(Ok(s)) => s,
                        other => {
                            if let Some(Err(e)) = other {
                                eprintln!("sep: AI separation unavailable ({}), using DSP", e);
                            }
                            crate::stems::separate(&pcm, &j2.pct)
                        }
                    };
                    if s.ai {
                        *j2.profile.lock().unwrap() = Some(Arc::new(crate::vibe::profile(&s, origin, bar, nbars)));
                    }
                    *j2.result.lock().unwrap() = Some(Arc::new(s));
                    j2.done.store(true, Ordering::SeqCst);
                });
                self.stem_job = Some((tid, job));
                break;
            }
        }
    }

    /// Per-song layer controls (vocals / music / bass / drums) for the playing and next song.
    pub fn layers_panel(&mut self, x: f32, y: f32, w: f32) -> f32 {
        let lib = self.lib.clone();
        let accent = self.acc;
        let mixer = self.mixer.clone();
        let a = self.snap.active;
        let decks = [a, 1 - a];
        let h = 62.0 + 2.0 * 140.0;
        rrect(x, y, w, h, 14.0, Color::new(1.0, 1.0, 1.0, 0.05));
        self.tx("layers", "LAYERS", x + 16.0, y + 26.0, 16.0, accent);
        let on = self.use_stems;
        if self.button(x + w - 136.0, y + 8.0, 120.0, 26.0, if on { "Layers: ON" } else { "Layers: OFF" }, on, accent) {
            self.use_stems = !self.use_stems;
        }
        draw_text("Estimated on this computer for the playing and next song only.", x + 16.0, y + 48.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.45));
        let mut yy = y + 68.0;
        for (n, &d) in decks.iter().enumerate() {
            let tid = self.snap.track[d];
            let label = match tid {
                Some(i) => format!("{}: {}", if n == 0 { "Playing" } else { "Up next" }, lib[i].title),
                None => "Empty deck".to_string(),
            };
            draw_text(&fit(&label, 18.0, w - 32.0), x + 16.0, yy, 18.0, WHITE);
            if self.snap.stems[d] {
                // Rotary knobs: drag around the dial (or click) to set the level.
                let ai = mixer.lock().unwrap().decks[d].stems.as_ref().map(|s| s.ai).unwrap_or(false);
                let order: &[usize] = if ai { &[0, 4, 1, 2, 3] } else { &[0, 1, 2, 3] };
                let cell = (w - 32.0) / order.len() as f32;
                let r = 20.0f32;
                let kc = yy + 12.0 + r + 8.0;
                let sweep = 2.356f32; // +-135 degrees
                for (slot, &k) in order.iter().enumerate() {
                    let v = (self.snap.sg[d][k] / 1.5).clamp(0.0, 1.0);
                    let cx = x + 16.0 + cell * (slot as f32 + 0.5);
                    let hv = self.hov(cx - r - 10.0, kc - r - 10.0, 2.0 * r + 20.0, 2.0 * r + 20.0);
                    let mut nv = v;
                    if hv && self.down {
                        let ang = (self.mx - cx).atan2(-(self.my - kc));
                        let ang = if ang.abs() > sweep { sweep * ang.signum() } else { ang };
                        nv = (ang + sweep) / (2.0 * sweep);
                        mixer.lock().unwrap().decks[d].sg[k] = nv * 1.5;
                    }
                    let seg = 28;
                    let pt = |a: f32, rr: f32| vec2(cx + rr * a.sin(), kc - rr * a.cos());
                    draw_circle(cx, kc, r - 3.0, Color::new(1.0, 1.0, 1.0, if hv { 0.10 } else { 0.06 }));
                    for i in 0..seg {
                        let a0 = -sweep + 2.0 * sweep * i as f32 / seg as f32;
                        let a1 = -sweep + 2.0 * sweep * (i + 1) as f32 / seg as f32;
                        let lit = (i as f32 + 0.5) / seg as f32 <= nv;
                        let c = if lit { accent } else { Color::new(1.0, 1.0, 1.0, 0.18) };
                        let (p0, p1) = (pt(a0, r), pt(a1, r));
                        draw_line(p0.x, p0.y, p1.x, p1.y, 4.0, c);
                    }
                    let ap = -sweep + 2.0 * sweep * nv;
                    let (p0, p1) = (pt(ap, 5.0), pt(ap, r - 5.0));
                    draw_line(p0.x, p0.y, p1.x, p1.y, 3.0, WHITE);
                    let lbl = crate::stems::NAMES[k];
                    draw_text(lbl, cx - tw(lbl, 15.0) / 2.0, kc + r + 17.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.75));
                    let pct = format!("{}", (nv * 150.0).round() as i32);
                    draw_text(&pct, cx - tw(&pct, 12.0) / 2.0, kc + 4.0, 12.0, Color::new(1.0, 1.0, 1.0, 0.7));
                }
                let ky = kc + r + 34.0;
                let bw = (w - 32.0 - 12.0) / 3.0;
                let presets: [(&str, [f32; crate::stems::NL]); 3] = [("Instrumental", [0.0, 1.0, 1.0, 1.0, 1.0]), ("Acapella", [1.0, 0.0, 0.0, 0.0, 0.0]), ("Reset", [1.0; crate::stems::NL])];
                for (pi, (name, vals)) in presets.iter().enumerate() {
                    if self.button(x + 16.0 + pi as f32 * (bw + 6.0), ky - 6.0, bw, 24.0, name, false, accent) {
                        mixer.lock().unwrap().decks[d].sg = *vals;
                    }
                }
            } else {
                let pct = match &self.stem_job {
                    Some((id, job)) if Some(*id) == tid => Some(job.pct.load(Ordering::Relaxed)),
                    _ => None,
                };
                let msg = if !self.use_stems {
                    "Layers are off".to_string()
                } else if let crate::sep::ModelState::Downloading(f) = crate::sep::model_state() {
                    format!("Downloading AI layer model... {:.0}%", f * 100.0)
                } else if let Some(p) = pct {
                    format!("Separating layers with AI... {}%", p)
                } else {
                    "Waiting to separate layers".to_string()
                };
                draw_text(&msg, x + 16.0, yy + 30.0, 17.0, Color::new(1.0, 1.0, 1.0, 0.6));
                if let Some(p) = pct {
                    rrect(x + 16.0, yy + 42.0, w - 32.0, 8.0, 4.0, Color::new(1.0, 1.0, 1.0, 0.15));
                    rrect(x + 16.0, yy + 42.0, ((w - 32.0) * p as f32 / 100.0).max(8.0), 8.0, 4.0, accent);
                }
            }
            yy += 140.0;
        }
        h
    }
}
