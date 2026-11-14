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
                let job = Arc::new(StemJob { pct: AtomicU32::new(0), done: AtomicBool::new(false), result: Mutex::new(None) });
                let pcm = self.lib[tid].pcm.clone();
                let j2 = job.clone();
                std::thread::spawn(move || {
                    let s = crate::stems::separate(&pcm, &j2.pct);
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
        let h = 62.0 + 2.0 * 178.0;
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
                let mut ky = yy + 22.0;
                for k in 0..4 {
                    let mut v = self.snap.sg[d][k] / 1.5;
                    draw_text(crate::stems::NAMES[k], x + 16.0, ky + 5.0, 16.0, Color::new(1.0, 1.0, 1.0, 0.75));
                    if self.slider(x + 90.0, ky, w - 130.0, &mut v, accent) {
                        mixer.lock().unwrap().decks[d].sg[k] = v * 1.5;
                    }
                    ky += 24.0;
                }
                let bw = (w - 32.0 - 12.0) / 3.0;
                let presets: [(&str, [f32; 4]); 3] = [("Instrumental", [0.0, 1.0, 1.0, 1.0]), ("Acapella", [1.0, 0.0, 0.0, 0.0]), ("Reset", [1.0; 4])];
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
                } else if let Some(p) = pct {
                    format!("Separating layers... {}%", p)
                } else {
                    "Waiting to separate layers".to_string()
                };
                draw_text(&msg, x + 16.0, yy + 30.0, 17.0, Color::new(1.0, 1.0, 1.0, 0.6));
                if let Some(p) = pct {
                    rrect(x + 16.0, yy + 42.0, w - 32.0, 8.0, 4.0, Color::new(1.0, 1.0, 1.0, 0.15));
                    rrect(x + 16.0, yy + 42.0, ((w - 32.0) * p as f32 / 100.0).max(8.0), 8.0, 4.0, accent);
                }
            }
            yy += 178.0;
        }
        h
    }
}
