//! Decks screen: a full DJ console beside Smart DJ.
//! Stacked scrolling waveforms on top, both decks with the mixer in the middle,
//! hot cues under each deck and a track browser to load songs onto A or B.
use crate::app::*;
use crate::tracks::{camelot_str, SR};
use crate::ui::*;
use crate::ui::draw_text;
use macroquad::prelude::*;

const WAVE_H: f32 = 96.0;
const EQ_W: f32 = 150.0;

impl App {
    pub fn decks_screen(&mut self, w: f32) {
        let pad = 24.0;
        let iw = w - 2.0 * pad;
        let mut y = HDR + 14.0 - self.scroll;

        // ---- 1. Stacked scrolling waveforms (deck A over deck B) ----
        rrect(pad, y, iw, WAVE_H * 2.0 + 10.0, 12.0, Color::new(0.02, 0.02, 0.03, 1.0));
        let ww = iw - 12.0 - EQ_W;
        for d in 0..2 {
            let ly = y + 5.0 + d as f32 * WAVE_H;
            self.scroll_wave(d, pad + 6.0, ly, ww, WAVE_H - 4.0);
            self.lane_eq(d, pad + 6.0 + ww, ly, EQ_W, WAVE_H - 4.0);
        }
        self.wave_rect = (pad, y, ww + 6.0, WAVE_H * 2.0 + 10.0);
        // shared centre line, like a DJ app's playhead
        let cx = pad + 6.0 + ww / 2.0;
        draw_line(cx, y + 2.0, cx, y + WAVE_H * 2.0 + 8.0, 2.0, Color::new(1.0, 1.0, 1.0, 0.9));
        draw_line(pad + 10.0, y + 5.0 + WAVE_H, pad + iw - 10.0, y + 5.0 + WAVE_H, 1.0, Color::new(1.0, 1.0, 1.0, 0.08));
        draw_line(pad + 6.0 + ww, y + 6.0, pad + 6.0 + ww, y + WAVE_H * 2.0 + 4.0, 1.0, Color::new(1.0, 1.0, 1.0, 0.1));
        let zl = format!("Zoom {:.0} s  (scroll to zoom)", self.wave_secs);
        draw_text(&zl, pad + 14.0, y + WAVE_H * 2.0 + 4.0, 12.0, Color::new(1.0, 1.0, 1.0, 0.35));
        if let (None, Some(ar)) = (self.snap.trans, self.armed.clone()) {
            let a = self.snap.active;
            let secs = ((ar.trigger - self.snap.pos[a]) / (SR as f64 * self.snap.rate[a].max(0.5))).max(0.0) as f32;
            let lib = self.lib.clone();
            let lab = format!("NEXT MIX in {}   {} > {}   {}   {} beats   into {}", mmss(secs), if a == 0 { "A" } else { "B" }, if a == 0 { "B" } else { "A" }, ar.style.name(), ar.beats, fit(&lib[ar.track].title, 13.0, 260.0));
            let bw = tw(&lab, 13.0) + 24.0;
            let bx = pad + 6.0 + ww / 2.0 - bw / 2.0;
            let by = y + WAVE_H * 2.0 - 8.0;
            let soon = secs < 10.0;
            let c = if soon { Color::new(1.0, 0.8, 0.3, 1.0) } else { Color::new(0.3, 1.0, 0.6, 1.0) };
            rrect(bx, by, bw, 18.0, 9.0, Color::new(0.0, 0.0, 0.0, 0.8));
            if soon {
                rrect(bx, by, bw, 18.0, 9.0, Color::new(c.r, c.g, c.b, 0.15 + 0.15 * (self.time * 5.0).sin().abs()));
            }
            draw_text(&lab, bx + 12.0, by + 13.0, 13.0, c);
        } else if self.snap.manual && self.snap.trans.is_none() {
            let lab = "MANUAL - no mix planned";
            draw_text(lab, pad + 6.0 + ww / 2.0 - tw(lab, 13.0) / 2.0, y + WAVE_H * 2.0 + 5.0, 13.0, Color::new(1.0, 1.0, 1.0, 0.4));
        }
        if let Some((prog, style, out, inn)) = self.snap.trans {
            let lab = format!("MIXING  {} > {}   {}   {:.0}%", if out == 0 { "A" } else { "B" }, if inn == 0 { "A" } else { "B" }, style.name(), prog * 100.0);
            let bw = tw(&lab, 13.0) + 24.0;
            let bx = pad + 6.0 + ww / 2.0 - bw / 2.0;
            let by = y + WAVE_H * 2.0 - 8.0;
            rrect(bx, by, bw, 18.0, 9.0, Color::new(0.0, 0.0, 0.0, 0.8));
            rrect(bx, by + 15.0, bw * prog, 3.0, 1.5, Color::new(0.3, 1.0, 0.6, 1.0));
            draw_text(&lab, bx + 12.0, by + 13.0, 13.0, Color::new(0.3, 1.0, 0.6, 1.0));
        }
        y += WAVE_H * 2.0 + 22.0;

        // ---- 2. Decks + mixer (shared with Smart DJ's pro view) ----
        let bottom = self.pro_decks(pad, y, iw);

        // ---- 3. Hot cues under each deck ----
        let narrow = iw < 900.0;
        let mut cy = bottom;
        if !narrow {
            let pw = (iw - 200.0) / 2.0;
            for d in 0..2 {
                let px = if d == 0 { pad } else { pad + pw + 200.0 };
                self.hot_cues(d, px, cy, pw);
            }
            cy += 58.0;
        } else {
            for d in 0..2 {
                self.hot_cues(d, pad, cy, iw);
                cy += 58.0;
            }
        }

        // ---- 4. Track browser ----
        let end = self.browser(pad, cy + 6.0, iw);
        self.content_h = end + self.scroll - HDR + 20.0;
    }

    /// One zoomed, scrolling waveform lane centred on the deck's playhead,
    /// with the beat grid and its cue points. Click to jump there.
    fn scroll_wave(&mut self, d: usize, x: f32, y: f32, w: f32, h: f32) {
        let lib = self.lib.clone();
        let label = if d == 0 { "A" } else { "B" };
        let Some(tid) = self.snap.track[d] else {
            draw_text(&format!("Deck {}  -  load a song from the list below", label), x + 14.0, y + h / 2.0 + 6.0, 16.0, Color::new(1.0, 1.0, 1.0, 0.35));
            return;
        };
        let t = &lib[tid];
        let accent = hsv(warm_hue(t.hue), 0.6, 1.0);
        let len = t.samples().max(1) as f64;
        let pos = self.smooth_pos(d, tid);
        let half = self.wave_secs as f64 / 2.0 * SR as f64;
        let (s0, s1) = (pos - half, pos + half);
        // Low / mid / high peaks for the song (cached once per song).
        let bands = match (self.band_cache.get(&tid), self.snap.pcm[d].clone()) {
            (Some(b), _) => Some(b.clone()),
            (None, Some(pcm)) => {
                let b = std::sync::Arc::new(crate::livemix::band_peaks(&pcm));
                self.band_cache.insert(tid, b.clone());
                Some(b)
            }
            _ => None,
        };
        let bb = crate::livemix::BAND_BLOCK as f64;
        let cols = ((w / 2.0) as usize).max(16);
        let cw = (w / cols as f32).max(1.0);
        let mid = y + h / 2.0;
        let p = &t.a.peaks;
        let per = (len / p.len().max(1) as f64).max(1.0);
        for i in 0..cols {
            let a = s0 + (s1 - s0) * i as f64 / cols as f64;
            let b = s0 + (s1 - s0) * (i + 1) as f64 / cols as f64;
            if b < 0.0 || a >= len {
                continue;
            }
            let played = b < pos;
            let alpha = if played { 0.5 } else { 1.0 };
            let cx_ = x + i as f32 * cw;
            if let Some(bands) = &bands {
                // Mixxx-style RGB wave: red = bass, green = mids, blue = highs.
                let ia = ((a.max(0.0) / bb) as usize).min(bands.len().saturating_sub(1));
                let ib = ((b / bb) as usize).max(ia + 1).min(bands.len());
                let (mut l, mut m, mut hi) = (0f32, 0f32, 0f32);
                for v in &bands[ia..ib] {
                    l = l.max(v[0]);
                    m = m.max(v[1]);
                    hi = hi.max(v[2]);
                }
                // lift mids/highs (they are quieter than bass) so colours stay vivid
                let eq = self.snap.eq[d];
                let tp = self.snap.tp[d];
                let (l, m, hi) = (l * eq[0] * tp.low, m * eq[1] * tp.mid, hi * eq[2] * tp.high);
                let (r, g, bl) = (l, m * 1.6, hi * 2.6);
                // saturate: push the weakest band toward zero so each bar has a clear colour
                let mn = r.min(g).min(bl);
                let (r, g, bl) = (r - mn * 0.85, g - mn * 0.85, bl - mn * 0.85);
                let mx = r.max(g).max(bl).max(1e-4);
                let col = Color::new((r / mx).powf(0.6), (g / mx).powf(0.6), (bl / mx).powf(0.6), alpha);
                let amp = (l * l + m * m + hi * hi).sqrt().min(1.0);
                let bh = (amp * h * 0.95).max(1.0);
                draw_rectangle(cx_, mid - bh / 2.0, cw, bh, col);
                // bright core for the highs, like Mixxx's inner layer
                let hh = (hi * 2.0).min(1.0) * h * 0.45;
                if hh > 1.0 {
                    draw_rectangle(cx_, mid - hh / 2.0, cw, hh, Color::new(1.0, 1.0, 1.0, 0.18 * alpha));
                }
            } else {
                let ia = ((a.max(0.0) / per) as usize).min(p.len().saturating_sub(1));
                let ib = ((b / per) as usize).max(ia + 1).min(p.len());
                let v = p[ia..ib].iter().fold(0f32, |m, &q| m.max(q));
                let bh = (v * h * 0.95).max(1.0);
                draw_rectangle(cx_, mid - bh / 2.0, cw, bh, Color::new(accent.r, accent.g, accent.b, alpha));
            }
        }
        // beat grid: thin lines, bar lines brighter
        if t.a.bpm > 30.0 {
            let spb = 60.0 / t.a.bpm as f64 * SR as f64;
            let o = t.a.origin;
            let mut k = ((s0 - o) / spb).ceil();
            while o + k * spb < s1 {
                let s = o + k * spb;
                if s >= 0.0 && s < len {
                    let bx = x + ((s - s0) / (s1 - s0)) as f32 * w;
                    let bar = (k as i64).rem_euclid(4) == 0;
                    draw_line(bx, y, bx, y + h, if bar { 1.5 } else { 1.0 }, Color::new(1.0, 1.0, 1.0, if bar { 0.35 } else { 0.12 }));
                }
                k += 1.0;
            }
        }
        // hot cues
        let cues = self.mixer.lock().unwrap().decks[d].cues;
        for (n, c) in cues.iter().enumerate() {
            if let Some(c) = c {
                if *c >= s0 && *c <= s1 {
                    let bx = x + ((*c - s0) / (s1 - s0)) as f32 * w;
                    draw_line(bx, y, bx, y + h, 2.0, cue_col(n));
                    rrect(bx, y + h - 16.0, 14.0, 16.0, 3.0, cue_col(n));
                    draw_text(&(n + 1).to_string(), bx + 3.0, y + h - 3.0, 14.0, BLACK);
                }
            }
        }
        // upcoming mix: shaded window on the outgoing deck, entry point on the incoming one
        if let Some(ar) = self.armed.clone() {
            if self.snap.trans.is_none() {
                let out = self.snap.active;
                let span = s1 - s0;
                let to_x = |s: f64| x + ((s - s0) / span) as f32 * w;
                let green = Color::new(0.3, 1.0, 0.6, 1.0);
                if d == out {
                    let spb = 60.0 / t.a.bpm.max(30.0) as f64 * SR as f64;
                    let end = ar.trigger + ar.beats as f64 * spb;
                    if end > s0 && ar.trigger < s1 {
                        let (a, b) = (to_x(ar.trigger.max(s0)), to_x(end.min(s1)));
                        draw_rectangle(a, y, b - a, h, Color::new(0.3, 1.0, 0.6, 0.12));
                        if ar.trigger >= s0 {
                            draw_line(a, y, a, y + h, 2.5, green);
                            rrect(a + 3.0, y + 28.0, 46.0, 16.0, 4.0, green);
                            draw_text("MIX IN", a + 8.0, y + 40.0, 12.0, BLACK);
                        }
                    }
                } else if self.snap.track[d] == Some(ar.track) && ar.in_pos >= s0 && ar.in_pos <= s1 {
                    let a = to_x(ar.in_pos);
                    draw_line(a, y, a, y + h, 2.5, green);
                    rrect(a + 3.0, y + 28.0, 52.0, 16.0, 4.0, green);
                    draw_text("ENTERS", a + 8.0, y + 40.0, 12.0, BLACK);
                }
            }
        }
        self.live_chips(d, x + 36.0, y + 6.0);
        // label and time
        rrect(x + 6.0, y + 6.0, 24.0, 20.0, 4.0, Color::new(accent.r, accent.g, accent.b, 0.85));
        draw_text(label, x + 13.0, y + 21.0, 16.0, BLACK);
        let secs = (pos / SR as f64) as f32;
        let left = ((len - pos) / SR as f64) as f32;
        let info = format!("{}   -{}   {:.2} BPM", mmss(secs), mmss(left), t.a.bpm * self.snap.rate[d] as f32);
        draw_text(&info, x + w - tw(&info, 14.0) - 8.0, y + 20.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.75));
        // click to jump within the visible window
        if self.click && self.hov(x, y, w, h) {
            let f = ((self.mx - x) / w) as f64;
            let target = (s0 + f * (s1 - s0)).clamp(0.0, len - 1.0);
            self.mixer.lock().unwrap().decks[d].pos = target;
            self.dirty = true;
        }
    }

    /// Small tags across the top of a lane naming every control the mix is moving right now.
    fn live_chips(&mut self, d: usize, x: f32, y: f32) {
        let tp = self.snap.tp[d];
        let db = |g: f32| if g < 0.02 { "kill".to_string() } else { format!("{:+.0} dB", 20.0 * g.max(0.001).log10()) };
        let mut tags: Vec<(String, Color)> = Vec::new();
        if self.snap.trans.is_some() {
            let gc = Color::new(1.0, 1.0, 1.0, 1.0);
            if (tp.gain - 1.0).abs() > 0.03 {
                tags.push((format!("VOL {:.0}%", tp.gain * 100.0), gc));
            }
            if (tp.low - 1.0).abs() > 0.03 {
                tags.push((format!("LOW {}", db(tp.low)), Color::new(1.0, 0.35, 0.35, 1.0)));
            }
            if (tp.mid - 1.0).abs() > 0.03 {
                tags.push((format!("MID {}", db(tp.mid)), Color::new(0.35, 1.0, 0.45, 1.0)));
            }
            if (tp.high - 1.0).abs() > 0.03 {
                tags.push((format!("HIGH {}", db(tp.high)), Color::new(0.45, 0.65, 1.0, 1.0)));
            }
            if tp.filt < 0.97 {
                tags.push((format!("FILTER {:.0}%", tp.filt * 100.0), Color::new(1.0, 0.75, 0.3, 1.0)));
            }
            if tp.echo > 0.03 {
                tags.push((format!("ECHO {:.0}%", tp.echo * 100.0), Color::new(0.8, 0.5, 1.0, 1.0)));
            }
            if tp.rev > 0.03 {
                tags.push((format!("REVERB {:.0}%", tp.rev * 100.0), Color::new(0.6, 0.8, 1.0, 1.0)));
            }
            for (k, g) in tp.stem.iter().enumerate() {
                if (*g - 1.0).abs() > 0.05 {
                    tags.push((format!("{} {:.0}%", crate::stems::NAMES[k].to_uppercase(), g * 100.0), Color::new(1.0, 0.85, 0.5, 1.0)));
                }
            }
        }
        if self.snap.loop_on[d] {
            tags.push(("LOOP".to_string(), Color::new(0.3, 1.0, 0.6, 1.0)));
        }
        if self.snap.sync[d] {
            tags.push(("SYNC".to_string(), Color::new(0.4, 0.85, 1.0, 1.0)));
        }
        if (self.snap.rate[d] - 1.0).abs() > 0.001 {
            tags.push((format!("TEMPO {:+.1}%", (self.snap.rate[d] - 1.0) * 100.0), Color::new(1.0, 1.0, 1.0, 0.85)));
        }
        let mut cx = x;
        for (t, c) in tags {
            let wd = tw(&t, 12.0) + 14.0;
            rrect(cx, y, wd, 18.0, 5.0, Color::new(0.0, 0.0, 0.0, 0.75));
            rrect(cx, y, wd, 18.0, 5.0, Color::new(c.r, c.g, c.b, 0.22));
            draw_text(&t, cx + 7.0, y + 13.0, 12.0, c);
            cx += wd + 4.0;
        }
    }

    /// HIGH / MID / LOW knobs beside a deck's waveform (drag up/down, right-click resets).
    fn lane_eq(&mut self, d: usize, x: f32, y: f32, w: f32, h: f32) {
        let names = ["LOW", "MID", "HIGH"];
        let cols = [Color::new(1.0, 0.3, 0.3, 1.0), Color::new(0.35, 1.0, 0.45, 1.0), Color::new(0.4, 0.6, 1.0, 1.0)];
        let cell = w / 3.0;
        let cy = y + h / 2.0 - 6.0;
        for k in 0..3 {
            let cx = x + cell * (k as f32 + 0.5);
            let tp = self.snap.tp[d];
            let auto = [tp.low, tp.mid, tp.high][k];
            let user = self.snap.eq[d][k];
            let g = (user * auto).min(1.5);
            let automated = (auto - 1.0).abs() > 0.03;
            let txt = if g < 0.02 { "kill".to_string() } else { format!("{:+.1}", 20.0 * g.max(0.001).log10()) };
            if let Some(nv) = self.knob(cx, cy, 16.0, g / 1.5, 1.0 / 1.5, names[k], &txt, cols[k]) {
                self.mixer.lock().unwrap().decks[d].eq[k] = nv * 1.5;
            }
            if automated {
                // the mix is turning this knob: glow ring + AUTO tag
                draw_circle_lines(cx, cy, 21.0, 2.0, Color::new(cols[k].r, cols[k].g, cols[k].b, 0.5 + 0.4 * (self.time * 6.0).sin().abs()));
                rrect(cx - 16.0, cy - 33.0, 32.0, 12.0, 4.0, cols[k]);
                draw_text("AUTO", cx - tw("AUTO", 11.0) / 2.0, cy - 24.0, 11.0, BLACK);
            }
        }
    }

    /// Playhead for drawing: advances every frame at the deck's speed and eases toward the
    /// engine position (which arrives in audio-buffer-sized steps), snapping only on real jumps.
    fn smooth_pos(&mut self, d: usize, tid: usize) -> f64 {
        let real = self.snap.pos[d];
        let (wt, mut p) = self.wave_pos[d];
        let running = self.snap.playing[d] && !self.snap.paused;
        if wt != Some(tid) || !running {
            p = real;
        } else {
            p += self.dt.min(0.1) as f64 * self.snap.rate[d] * SR as f64;
            let err = real - p;
            if err.abs() > 0.3 * SR as f64 {
                p = real; // seek, cue jump or loop wrap
            } else {
                p += err * 0.08;
            }
        }
        self.wave_pos[d] = (Some(tid), p);
        p
    }

    /// Four hot-cue pads: click an empty pad to set it, a set pad to jump there,
    /// right-click to clear.
    fn hot_cues(&mut self, d: usize, x: f32, y: f32, w: f32) {
        rrect(x, y, w, 48.0, 12.0, Color::new(1.0, 1.0, 1.0, 0.05));
        draw_text("HOT CUES", x + 14.0, y + 29.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.5));
        let has = self.snap.track[d].is_some();
        let cues = self.mixer.lock().unwrap().decks[d].cues;
        let bx0 = x + 100.0;
        let bw = ((w - 116.0) - 3.0 * 8.0) / 4.0;
        for n in 0..4 {
            let bx = bx0 + n as f32 * (bw + 8.0);
            let set = cues[n].is_some();
            let c = cue_col(n);
            let hv = self.hov(bx, y + 8.0, bw, 32.0);
            let fill = if set { Color::new(c.r, c.g, c.b, if hv { 1.0 } else { 0.85 }) } else { Color::new(1.0, 1.0, 1.0, if hv { 0.12 } else { 0.06 }) };
            rrect(bx, y + 8.0, bw, 32.0, 8.0, fill);
            let txt = match cues[n] {
                Some(p) => format!("{}  {}", n + 1, mmss((p / SR as f64) as f32)),
                None => format!("{}  set", n + 1),
            };
            draw_text(&txt, bx + bw / 2.0 - tw(&txt, 15.0) / 2.0, y + 29.0, 15.0, if set { BLACK } else { Color::new(1.0, 1.0, 1.0, 0.6) });
            if !has || !hv {
                continue;
            }
            if is_mouse_button_pressed(MouseButton::Right) {
                self.mixer.lock().unwrap().decks[d].cues[n] = None;
            } else if self.click {
                let mut m = self.mixer.lock().unwrap();
                let q = m.quantize;
                let dk = &mut m.decks[d];
                match dk.cues[n] {
                    Some(p) => {
                        dk.pos = p;
                        dk.need_reset = true;
                        if q && dk.playing {
                            m.snap_phase(d, false);
                        }
                    }
                    None => {
                        let (o, spb, p) = (dk.origin, dk.spb(), dk.pos);
                        dk.cues[n] = Some(if q { o + ((p - o) / spb).round() * spb } else { p });
                    }
                }
                self.dirty = true;
            }
        }
    }

    /// Library table with load-to-deck buttons.
    fn browser(&mut self, x: f32, y: f32, w: f32) -> f32 {
        let lib = self.lib.clone();
        let accent = self.acc;
        rrect(x, y, w, 40.0, 10.0, Color::new(1.0, 1.0, 1.0, 0.07));
        draw_text("TRACKS", x + 14.0, y + 26.0, 16.0, Color::new(1.0, 1.0, 1.0, 0.75));
        let hint = format!("{} songs  -  press A or B to load a deck", lib.len());
        draw_text(&hint, x + 100.0, y + 26.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.4));
        // columns
        let cols: [(&str, f32); 6] = [("Title", 0.0), ("Artist", 0.38), ("BPM", 0.62), ("Key", 0.70), ("Time", 0.77), ("Energy", 0.84)];
        let tx = x + 14.0;
        let tw_ = w - 14.0 - 110.0;
        let hy = y + 62.0;
        for (n, f) in cols.iter() {
            draw_text(n, tx + tw_ * f, hy, 13.0, Color::new(1.0, 1.0, 1.0, 0.45));
        }
        draw_line(x, hy + 8.0, x + w, hy + 8.0, 1.0, Color::new(1.0, 1.0, 1.0, 0.1));
        let mut order: Vec<usize> = (0..lib.len()).collect();
        order.sort_by(|a, b| lib[*b].a.bpm.partial_cmp(&lib[*a].a.bpm).unwrap_or(std::cmp::Ordering::Equal));
        let rh = 30.0;
        let mut ry = hy + 12.0;
        let screen_h = screen_height();
        let mut load: Option<(usize, usize)> = None;
        for (row, &i) in order.iter().enumerate() {
            if ry > screen_h || ry + rh < HDR {
                ry += rh;
                continue;
            }
            let t = &lib[i];
            let on_a = self.snap.track[0] == Some(i);
            let on_b = self.snap.track[1] == Some(i);
            let hv = self.hov(x, ry, w, rh);
            let bg = if on_a || on_b {
                Color::new(accent.r, accent.g, accent.b, 0.16)
            } else if hv {
                Color::new(1.0, 1.0, 1.0, 0.07)
            } else if row % 2 == 0 {
                Color::new(1.0, 1.0, 1.0, 0.025)
            } else {
                Color::new(0.0, 0.0, 0.0, 0.0)
            };
            draw_rectangle(x, ry, w, rh, bg);
            let ty = ry + 20.0;
            let col = hsv(warm_hue(t.hue), 0.55, 1.0);
            draw_rectangle(x, ry + 4.0, 3.0, rh - 8.0, col);
            draw_text(&fit(&t.title, 15.0, tw_ * 0.36), tx, ty, 15.0, WHITE);
            draw_text(&fit(&t.artist, 14.0, tw_ * 0.22), tx + tw_ * 0.38, ty, 14.0, Color::new(1.0, 1.0, 1.0, 0.6));
            draw_text(&format!("{:.1}", t.a.bpm), tx + tw_ * 0.62, ty, 14.0, Color::new(1.0, 1.0, 1.0, 0.85));
            draw_text(&camelot_str(t.a.key, t.a.minor), tx + tw_ * 0.70, ty, 14.0, col);
            draw_text(&mmss(t.samples() as f32 / SR as f32), tx + tw_ * 0.77, ty, 14.0, Color::new(1.0, 1.0, 1.0, 0.6));
            self.energy_bar(tx + tw_ * 0.84, ry + 12.0, tw_ * 0.14, t.energy, col);
            for d in 0..2 {
                let bx = x + w - 100.0 + d as f32 * 48.0;
                let lab = if d == 0 { "A" } else { "B" };
                let here = if d == 0 { on_a } else { on_b };
                if self.button(bx, ry + 3.0, 42.0, rh - 6.0, lab, here, accent) {
                    load = Some((d, i));
                }
            }
            ry += rh;
        }
        if let Some((d, i)) = load {
            self.load_track_to_deck(d, i);
        }
        ry + 10.0
    }

    /// Put any song on a deck (cued at its first beat, paused). A deck that is on air is left alone.
    pub(crate) fn load_track_to_deck(&mut self, d: usize, tid: usize) {
        if self.snap.active == d && self.snap.playing[d] && !self.snap.paused {
            self.say("That deck is on air - pause it or load the other deck");
            return;
        }
        let lib = self.lib.clone();
        let t = &lib[tid];
        let audio = t.audio();
        let cached = if self.use_stems { self.stem_cache.iter().find(|c| c.0 == tid).map(|c| c.1.clone()) } else { None };
        let mixer = self.mixer.clone();
        let mut m = mixer.lock().unwrap();
        let dk = &mut m.decks[d];
        if dk.track != Some(tid) || dk.stems.is_none() {
            dk.stems = cached;
        }
        dk.pcm = Some(audio);
        dk.track = Some(tid);
        dk.pos = t.a.origin;
        dk.rate = 1.0;
        dk.target_rate = 1.0;
        dk.bpm = t.a.bpm;
        dk.origin = t.a.origin;
        dk.norm = t.a.gain;
        dk.playing = false;
        dk.loop_on = false;
        dk.cues = [None; 4];
        dk.to_main = true;
        dk.to_dj = false;
        drop(m);
        self.dirty = true;
        let msg = format!("Loaded {} on deck {}", t.title, if d == 0 { "A" } else { "B" });
        self.say(&msg);
    }
}

fn cue_col(n: usize) -> Color {
    match n {
        0 => Color::new(1.0, 0.55, 0.15, 1.0),
        1 => Color::new(0.3, 0.85, 1.0, 1.0),
        2 => Color::new(0.45, 1.0, 0.5, 1.0),
        _ => Color::new(1.0, 0.4, 0.75, 1.0),
    }
}
