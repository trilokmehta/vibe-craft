//! Live mix tools: beat zoom (zoomed, beat-gridded view of both songs with a
//! phase meter), snap to beat, and quantized jumps.
use crate::app::*;
use crate::tracks::SR;
use crate::ui::*;
use crate::ui::draw_text;
use macroquad::prelude::*;

impl App {
    /// Live mix toolbar buttons (right side of the LIVE MIX header).
    pub fn live_tools(&mut self, x: f32, y: f32, w: f32, accent: Color) {
        let bw = 138.0;
        let q = self.snap.quantize;
        if self.button(x + w - bw, y, bw, 26.0, if q { "Snap to beat: ON" } else { "Snap to beat: OFF" }, q, accent) {
            let mut m = self.mixer.lock().unwrap();
            m.quantize = !m.quantize;
            let on = m.quantize;
            drop(m);
            self.say(if on { "Snap to beat on: jumps and starts land on the beat, so the groove carries on" } else { "Snap to beat off: jumps go exactly where you click" });
        }
        let z = self.zoom;
        if self.button(x + w - 2.0 * bw - 8.0, y, bw, 26.0, if z { "Beat zoom: ON" } else { "Beat zoom" }, z, accent) {
            self.zoom = !self.zoom;
        }
    }

    /// Jump deck `d` to `target`. With Snap to beat the jump is a whole number of
    /// beats, so the beat carries on exactly where it was.
    pub fn jump_deck(&mut self, d: usize, target: f64) {
        let tid = match self.snap.track[d] {
            Some(t) => t,
            None => return,
        };
        let bpm = self.lib[tid].a.bpm;
        let mixer = self.mixer.clone();
        let mut m = mixer.lock().unwrap();
        let len = m.decks[d].len();
        let cur = m.decks[d].pos;
        let p = if m.quantize {
            let spb = SR as f64 * 60.0 / bpm.max(40.0) as f64;
            cur + ((target - cur) / spb).round() * spb
        } else {
            target
        };
        m.decks[d].pos = p.clamp(0.0, (len - 1.0).max(0.0));
        drop(m);
        self.dirty = true;
    }

    /// Zoomed, beat-gridded view of the playing and the next song on one time scale:
    /// when the beats are locked their grid lines stand on top of each other.
    pub fn beat_zoom(&mut self, x: f32, y: f32, w: f32, accent: Color) -> f32 {
        let lib = self.lib.clone();
        let lane = 74.0;
        let h = 70.0 + 2.0 * (lane + 24.0);
        rrect(x, y, w, h, 14.0, Color::new(1.0, 1.0, 1.0, 0.05));
        self.zoom_rect = (x, y, w, h);
        self.tx("zoom", "BEAT ZOOM", x + 16.0, y + 26.0, 16.0, accent);
        draw_text(&fit(&format!("{:.1} s shown - mouse wheel zooms, click jumps, drag scrubs (playing) or sets the entry (next)", self.zoom_secs), 14.0, w - 230.0), x + 120.0, y + 26.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.5));
        let bw = 34.0;
        if self.button(x + w - 16.0 - bw, y + 8.0, bw, 26.0, "+", false, accent) {
            self.zoom_secs = (self.zoom_secs * 0.8).max(0.5);
        }
        if self.button(x + w - 16.0 - 2.0 * bw - 6.0, y + 8.0, bw, 26.0, "-", false, accent) {
            self.zoom_secs = (self.zoom_secs * 1.25).min(32.0);
        }
        let a = self.snap.active;
        let inn = 1 - a;
        // Phase meter: how far the next song's beat is from the playing song's beat.
        let my = y + 50.0;
        let mw = (w - 32.0).min(380.0);
        let mx0 = x + 16.0;
        let ph = if self.snap.playing[inn] && self.snap.playing[a] { self.snap.phase_ms[inn] } else { None };
        draw_text("PHASE", mx0, my + 5.0, 13.0, Color::new(1.0, 1.0, 1.0, 0.5));
        let bx = mx0 + 52.0;
        let bwid = mw - 52.0;
        rrect(bx, my - 4.0, bwid, 10.0, 5.0, Color::new(1.0, 1.0, 1.0, 0.12));
        draw_line(bx + bwid / 2.0, my - 7.0, bx + bwid / 2.0, my + 9.0, 1.5, Color::new(1.0, 1.0, 1.0, 0.5));
        match ph {
            Some(ms) => {
                let bpm = self.snap.track[a].map(|t| lib[t].a.bpm).unwrap_or(120.0);
                let beat_ms = 60000.0 / (bpm * self.snap.rate[a] as f32).max(40.0);
                let f = (ms / (beat_ms / 2.0)).clamp(-1.0, 1.0);
                let c = if ms.abs() < 12.0 {
                    Color::new(0.3, 1.0, 0.6, 1.0)
                } else if ms.abs() < 40.0 {
                    Color::new(1.0, 0.8, 0.2, 1.0)
                } else {
                    Color::new(1.0, 0.35, 0.35, 1.0)
                };
                draw_circle(bx + bwid / 2.0 + f * bwid / 2.0, my + 1.0, 7.0, c);
                let lock = if self.snap.sync[inn] { "  LOCKED" } else { "" };
                draw_text(&format!("{:+.0} ms{}", ms, lock), bx + bwid + 10.0, my + 6.0, 15.0, c);
            }
            None => {
                draw_text("shows live while both songs play", bx + bwid + 10.0, my + 6.0, 13.0, Color::new(1.0, 1.0, 1.0, 0.4));
            }
        }
        let mut ly = y + 70.0;
        let lw = w - 32.0;
        let lx = x + 16.0;
        let nxt = self.queue.first().map(|q| q.track);
        for (row, d) in [a, inn].into_iter().enumerate() {
            let tid = if row == 0 { self.snap.track[d] } else { self.snap.track[d].filter(|t| Some(*t) == nxt || self.snap.playing[d]) };
            let label = match tid {
                Some(t) => format!("{}  {}", if row == 0 { "PLAYING" } else { "UP NEXT" }, lib[t].title),
                None => (if row == 0 { "PLAYING" } else { "UP NEXT - not loaded yet" }).to_string(),
            };
            draw_text(&fit(&label, 15.0, lw), lx, ly + 12.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.7));
            let wy = ly + 18.0;
            rrect(lx, wy, lw, lane, 6.0, Color::new(0.0, 0.0, 0.0, 0.25));
            let (t, pcm) = match (tid, self.snap.pcm[d].clone()) {
                (Some(t), Some(p)) => (t, p),
                _ => {
                    ly += lane + 24.0;
                    continue;
                }
            };
            let tr = &lib[t];
            let tint = hsv(tr.hue / 360.0, 0.45, 1.0);
            let rate = self.snap.rate[d].max(0.1);
            let live = row == 0 || self.snap.playing[d];
            let mut center = if live { self.snap.pos[d] } else { self.armed.as_ref().filter(|ar| ar.track == t).map(|ar| ar.in_pos).unwrap_or(tr.a.origin) };
            // Same seconds on screen for both lanes (playback time), so locked beats line up.
            let span = self.zoom_secs as f64 * SR as f64 * rate;
            // Scrub: grab the waveform and drag it, like a record. It follows the mouse exactly.
            let hv = self.hov(lx, wy, lw, lane);
            if self.click && hv {
                self.zoom_drag = Some((d, self.mx, center, false, self.mx));
            }
            let mut released: Option<(f64, bool)> = None;
            if let Some((dd, last, sp, moved, grab)) = self.zoom_drag {
                if dd == d {
                    let moved = moved || (self.mx - grab).abs() > 4.0;
                    let maxp = (pcm.len() as f64 - 1.0).max(0.0);
                    if self.down {
                        // Relative nudges: the song keeps playing, each mouse move shifts it a little.
                        let mut target = sp;
                        if moved {
                            target = (sp - ((self.mx - last) / lw) as f64 * span).clamp(0.0, maxp);
                            if live {
                                if target != sp {
                                    self.mixer.lock().unwrap().nudge_pos(d, target - sp);
                                }
                            } else {
                                center = target;
                            }
                        }
                        self.zoom_drag = Some((dd, self.mx, target, moved, grab));
                    } else {
                        self.zoom_drag = None;
                        released = Some((sp, moved));
                    }
                }
            }
            let s0 = center - span / 2.0;
            let cols = ((lw / 2.0) as usize).max(16);
            let per = span / cols as f64;
            let step = ((per / 48.0) as usize).max(1);
            // Bars sit on a fixed grid in song time and slide by the sub-column remainder,
            // so they do not shimmer while the view moves.
            let s0q = (s0 / per).floor() * per;
            let shift = ((s0 - s0q) / per) as f32 * lw / cols as f32;
            for c in 0..=cols {
                let a0 = s0q + c as f64 * per;
                let (i0, i1) = (a0.max(0.0) as usize, ((a0 + per).max(0.0) as usize).min(pcm.len()));
                if i0 >= i1 {
                    continue;
                }
                let mut pk = 0f32;
                let mut i = i0;
                while i < i1 {
                    pk = pk.max(pcm[i].abs());
                    i += step;
                }
                let bh = ((pk * tr.a.gain).min(1.0) * lane * 0.92).max(1.0);
                let alpha = if a0 < center { 0.45 } else { 0.85 };
                let bx = lx + c as f32 * lw / cols as f32 - shift;
                let bwid = (lw / cols as f32 - 0.5).max(1.0);
                let (x0, x1) = (bx.max(lx), (bx + bwid).min(lx + lw));
                if x1 > x0 {
                    draw_rectangle(x0, wy + (lane - bh) / 2.0, x1 - x0, bh, Color::new(tint.r, tint.g, tint.b, alpha));
                }
            }
            // Beat grid: bars bright with their number, beats faint.
            let spb = SR as f64 * 60.0 / tr.a.bpm.max(40.0) as f64;
            let b_first = ((s0 - tr.a.origin) / spb).ceil() as i64;
            let b_last = ((s0 + span - tr.a.origin) / spb).floor() as i64;
            let px_per_beat = lw as f64 * spb / span;
            if b_last - b_first < 600 {
                for b in b_first..=b_last {
                    let bp = tr.a.origin + b as f64 * spb;
                    let gx = lx + ((bp - s0) / span) as f32 * lw;
                    let bar = b.rem_euclid(4) == 0;
                    if !bar && px_per_beat < 6.0 {
                        continue;
                    }
                    draw_line(gx, wy, gx, wy + lane, if bar { 2.0 } else { 1.0 }, Color::new(1.0, 1.0, 1.0, if bar { 0.55 } else { 0.18 }));
                    if bar && px_per_beat * 4.0 > 34.0 {
                        draw_text(&format!("{}", b.div_euclid(4) + 1), gx + 3.0, wy + 12.0, 12.0, Color::new(1.0, 1.0, 1.0, 0.6));
                    }
                }
            }
            // Playhead (or the planned entry) in the middle.
            let cx = lx + lw / 2.0;
            draw_line(cx, wy - 3.0, cx, wy + lane + 3.0, 2.5, if live { WHITE } else { Color::new(0.3, 1.0, 0.6, 1.0) });
            if let Some((target, moved)) = released {
                let end = (tr.a.origin + tr.a.last_bar as f64 * 4.0 * spb).min(tr.samples() as f64);
                if moved {
                    if live {
                        // let go: resume there, back on the other deck's beat when Snap is on
                        let mut m = self.mixer.lock().unwrap();
                        m.scrub_end(d);
                        if m.quantize {
                            m.snap_phase(d, false);
                        }
                    } else {
                        let p = self.snap_bar(t, target);
                        self.set_section(t, p, end);
                    }
                } else {
                    // a plain click: jump / set the entry where the mouse is
                    let p = s0 + ((self.mx - lx) / lw) as f64 * span;
                    if live {
                        self.jump_deck(d, p);
                    } else {
                        self.set_section(t, p, end);
                    }
                }
                if !live {
                    self.dirty = true;
                }
            }
            ly += lane + 24.0;
        }
        h
    }

    /// Compact up-next queue on the DJ screen: change the next song without leaving the mix.
    pub fn queue_panel(&mut self, x: f32, y: f32, w: f32, accent: Color, max_rows: usize) -> f32 {
        let lib = self.lib.clone();
        let rows = self.queue.len().min(max_rows);
        let rh = 46.0;
        let h = 52.0 + rows as f32 * rh + 8.0;
        rrect(x, y, w, h, 14.0, Color::new(1.0, 1.0, 1.0, 0.05));
        self.tx("qpanel", "QUEUE", x + 16.0, y + 26.0, 16.0, accent);
        draw_text("Mix now = go there now, Next = play after this song", x + 80.0, y + 26.0, 13.0, Color::new(1.0, 1.0, 1.0, 0.45));
        let mut action: Option<(usize, u8)> = None;
        let mut yy = y + 44.0;
        for i in 0..rows {
            let t = &lib[self.queue[i].track];
            let bw = [70.0f32, 48.0, 28.0];
            let btns = bw.iter().sum::<f32>() + 12.0;
            rrect(x + 10.0, yy, w - 20.0, rh - 6.0, 8.0, Color::new(1.0, 1.0, 1.0, if i == 0 { 0.10 } else { 0.04 }));
            let tw_ = w - 40.0 - btns;
            draw_text(&fit(&format!("{}. {}", i + 1, t.title), 16.0, tw_), x + 18.0, yy + 18.0, 16.0, WHITE);
            draw_text(&fit(&format!("{}  {:.0} BPM  {}", t.artist, t.a.bpm, crate::tracks::camelot_str(t.a.key, t.a.minor)), 13.0, tw_), x + 18.0, yy + 34.0, 13.0, Color::new(1.0, 1.0, 1.0, 0.55));
            let mut bx = x + w - 14.0 - btns;
            for (k, l) in ["Mix now", "Next", "x"].iter().enumerate() {
                if (k != 1 || i > 0) && self.button(bx, yy + 7.0, bw[k], 26.0, l, false, accent) {
                    action = Some((i, k as u8));
                }
                bx += bw[k] + 6.0;
            }
            yy += rh;
        }
        if let Some((i, k)) = action {
            match k {
                0 => {
                    let t = self.queue[i].track;
                    self.play_now(t);
                }
                1 => {
                    let it = self.queue.remove(i);
                    self.queue.insert(0, it);
                    self.say("Moved to next - the DJ mixes into it after this song");
                }
                _ => {
                    self.queue.remove(i);
                    self.refill();
                }
            }
            self.dirty = true;
        }
        h
    }
}
