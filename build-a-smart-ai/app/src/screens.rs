//! Smart DJ screen (with pro decks), queue screen and library screen.
use crate::app::*;
use crate::dj::*;
use crate::engine::Style;
use crate::tracks::{camelot_str, key_name, SR};
use crate::ui::*;
use crate::ui::{draw_text, measure_text};
use macroquad::prelude::*;

impl App {
    fn track_card(&mut self, x: f32, y: f32, w: f32, label: &str, id: usize, eff: f32) {
        let lib = self.lib.clone();
        let t = &lib[id];
        let accent = hsv(warm_hue(t.hue), 0.55, 1.0);
        // compact card (108 px) so the whole Smart DJ screen fits without scrolling
        rrect(x, y, w, 108.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.07));
        art(x + 12.0, y + 14.0, 80.0, t);
        let tx = x + 104.0;
        let mw = w - 116.0;
        draw_text(label, tx, y + 20.0, 14.0, accent);
        draw_text(&fit(&t.title, 22.0, mw), tx, y + 44.0, 22.0, WHITE);
        draw_text(&fit(&t.artist, 16.0, mw), tx, y + 63.0, 16.0, Color::new(1.0, 1.0, 1.0, 0.65));
        draw_text(&format!("{:.1} BPM", eff), tx, y + 88.0, 20.0, accent);
        draw_text(&format!("{} / {}", camelot_str(t.a.key, t.a.minor), key_name(t.a.key, t.a.minor)), tx + 100.0, y + 88.0, 16.0, WHITE);
        self.energy_bar(tx, y + 96.0, mw, t.energy, accent);
    }

    pub fn smart_dj(&mut self, w: f32) {
        let lib = self.lib.clone();
        let narrow = w < 960.0;
        let pad = 32.0;
        let (lx, lw) = if narrow { (pad, w - 2.0 * pad) } else { (pad, w * 0.58 - pad) };
        let mut y = HDR + 18.0 - self.scroll;
        let a = self.snap.active;
        let cur = self.current;
        let t = &lib[cur];
        let accent = hsv(warm_hue(t.hue), 0.55, 1.0);
        let eff_cur = t.a.bpm * self.snap.rate[a] as f32;
        let nxt = self.queue.first().map(|q| q.track);
        let cw = (lw - 24.0) / 2.0;
        self.track_card(lx, y, cw, "NOW PLAYING", cur, eff_cur);
        self.mark("dj_cards");
        if let Some(n) = nxt {
            let eff_n = match &self.armed {
                Some(ar) => lib[n].a.bpm * ar.in_rate as f32,
                None => lib[n].a.bpm,
            };
            self.track_card(lx + cw + 24.0, y, cw, "UP NEXT", n, eff_n);
        }
        y += 128.0;
        rrect(lx, y, lw, 376.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.05));
        self.tx("mix", "LIVE MIX", lx + 16.0, y + 26.0, 16.0, accent);
        draw_text(&fit("Drag on a song to play only that section. Click a point chip to choose where to mix.", 14.0, lw - 420.0), lx + 100.0, y + 26.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.5));
        self.live_tools(lx, y + 8.0, lw - 16.0, accent);
        let ww = lw - 32.0;
        let wx = lx + 16.0;
        let cur_pts = top_out_points(t, self.snap.pos[a], 8);
        let mut marks_cur: Vec<(f64, Color)> = Vec::new();
        for p in &cur_pts {
            marks_cur.push((p.pos, Color::new(1.0, 0.7, 0.2, 0.55)));
        }
        if let Some(ar) = &self.armed {
            marks_cur.push((ar.trigger, Color::new(0.3, 1.0, 0.6, 0.95)));
            marks_cur.push((ar.trigger + ar.l_trk, Color::new(1.0, 0.3, 0.4, 0.8)));
        }
        self.wave(wx, y + 44.0, ww, 70.0, t, Some(self.snap.pos[a]), &marks_cur, accent);
        self.section_overlay(wx, y + 44.0, ww, 70.0, cur, accent);
        let on_btn = self.sections.get(cur).copied().flatten().is_some() && self.hov(wx + ww - 130.0, y + 90.0, 124.0, 22.0);
        let clk = self.click;
        if on_btn {
            self.click = false;
        }
        self.wave_interact(wx, y + 44.0, ww, 70.0, cur, 0);
        self.click = clk;
        let chosen_out = self.armed.as_ref().map(|ar| ar.trigger);
        let tol = 2.0 * SR as f64 * 60.0 / t.a.bpm as f64 * 4.0;
        if let Some(i) = self.point_chips(wx, y + 122.0, ww, "Exit points", &cur_pts, chosen_out, tol, accent) {
            self.forced_out = Some((cur, cur_pts[i].pos));
            self.dirty = true;
            self.say("Exit point chosen: the DJ will mix out there");
        }
        if self.sections.get(cur).copied().flatten().is_some() && self.button(wx + ww - 130.0, y + 90.0, 124.0, 22.0, "Clear section", false, accent) {
            self.clear_section(cur);
        }
        if let Some(n) = nxt {
            let tn = &lib[n];
            let an = hsv(warm_hue(tn.hue), 0.55, 1.0);
            let inn = 1 - a;
            let ph = if self.snap.playing[inn] { Some(self.snap.pos[inn]) } else { None };
            let in_at = self.armed.as_ref().map(|a| a.in_pos).unwrap_or(tn.a.origin);
            let next_pts = top_in_points(tn, 8);
            let mut marks: Vec<(f64, Color)> = Vec::new();
            for p in &next_pts {
                marks.push((p.pos, Color::new(0.4, 0.8, 1.0, 0.55)));
            }
            marks.push((in_at, Color::new(0.3, 1.0, 0.6, 0.95)));
            if let Some((ct, cp)) = self.mixer.lock().unwrap().cue_position() {
                if ct == n {
                    marks.push((cp, Color::new(1.0, 0.4, 0.9, 1.0)));
                }
            }
            self.wave(wx, y + 160.0, ww, 70.0, tn, ph.or(Some(in_at)), &marks, an);
            self.section_overlay(wx, y + 160.0, ww, 70.0, n, an);
            // The Clear section button sits on the wave: its clicks must not start a new selection.
            let on_btn = self.sections.get(n).copied().flatten().is_some() && self.hov(wx + ww - 130.0, y + 206.0, 124.0, 22.0);
            let clk = self.click;
            if on_btn {
                self.click = false;
            }
            self.wave_interact(wx, y + 160.0, ww, 70.0, n, 1);
            self.click = clk;
            let tol_n = 2.0 * SR as f64 * 60.0 / tn.a.bpm as f64 * 4.0;
            if let Some(i) = self.point_chips(wx, y + 238.0, ww, "Entry points", &next_pts, Some(in_at), tol_n, an) {
                let end = (tn.a.origin + tn.a.last_bar as f64 * tol_n / 2.0).min(tn.samples() as f64);
                self.set_section(n, next_pts[i].pos, end);
            }
            if self.sections.get(n).copied().flatten().is_some() && self.button(wx + ww - 130.0, y + 206.0, 124.0, 22.0, "Clear section", false, an) {
                self.clear_section(n);
            }
        }
        let (prog, label) = if let Some((p, st, _, _)) = self.snap.trans {
            (p, format!("Mixing now: {}", st.name()))
        } else if let Some(ar) = &self.armed {
            let secs = ((ar.trigger - self.snap.pos[a]) / (SR as f64 * self.snap.rate[a].max(0.1))).max(0.0) as f32;
            (0.0, format!("{} in {} ({} beats)", ar.style.name(), mmss(secs), ar.beats))
        } else {
            (0.0, "Waiting for next track".to_string())
        };
        self.tx("transition", &fit(&label, 20.0, ww), wx, y + 292.0, 20.0, WHITE);
        rrect(wx, y + 302.0, ww, 10.0, 5.0, Color::new(1.0, 1.0, 1.0, 0.15));
        if prog > 0.0 {
            rrect(wx, y + 302.0, (ww * prog).max(10.0), 10.0, 5.0, accent);
        }
        let detail = match &self.armed {
            Some(ar) if self.snap.trans.is_none() => {
                if ar.matched {
                    format!("AutoMix: {}  |  tempo {:+.1}%, key fit {:.0}%", ar.why, (ar.in_rate - 1.0) * 100.0, ar.ks * 100.0)
                } else if self.set.auto && ar.style == Style::EchoOut {
                    format!("AutoMix: {}  |  tempos too far apart to beat-match, using an echo transition", ar.why)
                } else {
                    format!("AutoMix: {}", ar.why)
                }
            }
            _ => format!("Crossfader {:.0}%  |  Deck {} on air", self.snap.xf * 100.0, if a == 0 { "A" } else { "B" }),
        };
        draw_text(&fit(&detail, 16.0, ww), wx, y + 332.0, 16.0, Color::new(1.0, 1.0, 1.0, 0.6));
        draw_text("Orange: possible exits   Blue: possible entries   Green: chosen mix start   Red: mix end   Shaded: selected section   Pink: DJ preview (Ctrl+drag the lower wave)", wx, y + 358.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.4));
        y += 408.0;
        if self.zoom {
            y += self.beat_zoom(lx, y, lw, accent) + 24.0;
        }
        if let Some(sc) = self.armed.as_ref().and_then(|ar| ar.script) {
            y = self.layer_timeline(&sc, prog, lx, y, lw, accent);
        }
        let show_why = narrow || !self.advanced;
        if show_why {
        rrect(lx, y, lw, 190.0, 14.0, Color::new(accent.r, accent.g, accent.b, 0.10));
        self.tx("explain", "WHY THIS TRACK NEXT", lx + 16.0, y + 26.0, 16.0, accent);
        let mut ey = y + 52.0;
        if let Some(q) = self.queue.first() {
            let head = format!("{} by {}", lib[q.track].title, lib[q.track].artist);
            draw_text(&fit(&head, 22.0, lw - 32.0), lx + 16.0, ey, 22.0, WHITE);
            ey += 8.0;
            let reasons: Vec<String> = q.reason.split(" | ").map(|s| s.to_string()).collect();
            for r in reasons.iter().take(6) {
                for (i, l) in wrap(r, 17.0, lw - 56.0).iter().enumerate() {
                    ey += 21.0;
                    if i == 0 {
                        draw_circle(lx + 22.0, ey - 5.0, 3.0, accent);
                    }
                    draw_text(l, lx + 34.0, ey, 17.0, Color::new(1.0, 1.0, 1.0, 0.82));
                }
                if ey > y + 182.0 {
                    break;
                }
            }
        }
        y += 218.0;
        }
        let left_end = y;
        let (rx, rw) = if narrow { (pad, w - 2.0 * pad) } else { (w * 0.58 + 8.0, w * 0.42 - pad - 8.0) };
        let ry0 = if narrow { left_end } else { HDR + 18.0 - self.scroll };
        let sw = rw - 32.0;
        let adv = self.advanced;
        // how many mood-chip rows fit
        let mut mrows = 1.0f32;
        let mut mxp = 0.0f32;
        for m in MOODS {
            let wd = tw(m.name(), 18.0) + 22.0;
            if mxp + wd > sw {
                mrows += 1.0;
                mxp = 0.0;
            }
            mxp += wd + 6.0;
        }
        let ch = 40.0 + 46.0 + 16.0 + mrows * 36.0 + 3.0 * 46.0 + 38.0 + 54.0 + 4.0;
        rrect(rx, ry0, rw, ch, 14.0, Color::new(1.0, 1.0, 1.0, 0.05));
        self.tx("controls", "DJ CONTROLS", rx + 16.0, ry0 + 26.0, 16.0, accent);
        let mut ry = ry0 + 40.0;
        // one row: Auto DJ | genre | pro decks
        let third = (sw - 12.0) / 3.0;
        let auto = self.set.auto;
        if self.button(rx + 16.0, ry, third, 34.0, if auto { "AUTO DJ  ON" } else { "AUTO DJ  OFF" }, auto, accent) {
            self.set.auto = !self.set.auto;
            self.dirty = true;
        }
        let gl = self.set.genre.clone().unwrap_or_else(|| "Any genre".to_string());
        if self.button(rx + 16.0 + third + 6.0, ry, third, 34.0, &gl, self.set.genre.is_some(), accent) {
            let idx = match &self.set.genre {
                None => 0,
                Some(g) => self.genres.iter().position(|x| x == g).map(|p| p + 1).unwrap_or(0),
            };
            self.set.genre = if idx < self.genres.len() { Some(self.genres[idx].clone()) } else { None };
            self.rebuild_queue();
        }
        if self.button(rx + 16.0 + 2.0 * (third + 6.0), ry, third, 34.0, if adv { "Hide pro decks" } else { "Pro decks" }, adv, accent) {
            self.advanced = !self.advanced;
        }
        ry += 46.0;
        draw_text("MOOD", rx + 16.0, ry + 8.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.5));
        ry += 16.0;
        let mut mx = rx + 16.0;
        for m in MOODS {
            let wd = tw(m.name(), 18.0) + 22.0;
            if mx + wd > rx + rw - 16.0 {
                mx = rx + 16.0;
                ry += 36.0;
            }
            let on = self.set.mood == m;
            if self.button(mx, ry, wd, 30.0, m.name(), on, hsv(mood_hue(m), 0.5, 1.0)) {
                self.set.mood = m;
                self.rebuild_queue();
            }
            mx += wd + 6.0;
        }
        ry += 36.0 + 12.0;
        let mut v = self.set.energy;
        draw_text(&format!("Energy: {:.0}%  (target {:.0}%)", v * 100.0, self.set.target_energy() * 100.0), rx + 16.0, ry, 16.0, WHITE);
        if self.slider(rx + 16.0, ry + 12.0, sw, &mut v, accent) {
            self.set.energy = v;
            self.rebuild_queue();
        }
        ry += 46.0;
        let mut v = self.set.intensity;
        let iname = if v < 0.34 {
            "long & smooth"
        } else if v < 0.67 {
            "balanced"
        } else {
            "short & punchy"
        };
        draw_text(&format!("Transition intensity: {}", iname), rx + 16.0, ry, 16.0, WHITE);
        if self.slider(rx + 16.0, ry + 12.0, sw, &mut v, accent) {
            self.set.intensity = v;
            self.dirty = true;
        }
        ry += 46.0;
        let lb_label = if self.set.layer_build { "Layer build: ON" } else { "Layer build: off" };
        if self.button(rx + 16.0, ry - 8.0, sw.min(240.0), 30.0, lb_label, self.set.layer_build, accent) {
            self.set.layer_build = !self.set.layer_build;
            self.dirty = true;
        }
        let hint = fit("music > melody > drums+bass > vocals", 14.0, (sw - sw.min(240.0) - 12.0).max(40.0));
        draw_text(&hint, rx + 16.0 + sw.min(240.0) + 12.0, ry + 12.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.5));
        ry += 38.0;
        draw_text("TRANSITION WEIGHTS", rx + 16.0, ry + 8.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.5));
        ry += 16.0;
        let half = (sw - 6.0) / 2.0;
        if self.button(rx + 16.0, ry, half, 30.0, "Profile 1", self.set.profile == TransitionProfile::V1, accent) {
            self.set.profile = TransitionProfile::V1;
            self.rebuild_queue();
        }
        if self.button(rx + 16.0 + half + 6.0, ry, half, 30.0, "Profile 2", self.set.profile == TransitionProfile::V2, accent) {
            self.set.profile = TransitionProfile::V2;
            self.rebuild_queue();
        }
        ry += 38.0;
        let mut v = self.set.discovery;
        draw_text(&format!("Familiar {:.0}%  <->  Discovery {:.0}%", (1.0 - v) * 100.0, v * 100.0), rx + 16.0, ry, 16.0, WHITE);
        if self.slider(rx + 16.0, ry + 12.0, sw, &mut v, accent) {
            self.set.discovery = v;
            self.rebuild_queue();
        }
        let mut rb = ry0 + ch + 18.0;
        // Layers take a lot of room: shown when the pro decks are hidden.
        if !adv || narrow {
            let lh = self.layers_panel(rx, rb, rw);
            rb += lh + 18.0;
        }
        // The queue takes whatever height is left, so nothing needs scrolling.
        let limit = if adv && !narrow { left_end } else { screen_height() - FTR - 8.0 };
        let qrows = (((limit - rb) - 60.0) / 46.0).floor().clamp(2.0, 8.0) as usize;
        rb += self.queue_panel(rx, rb, rw, accent, qrows) + 18.0;
        let ry = rb;
        let mut bottom = left_end.max(ry);
        if self.advanced {
            bottom = self.pro_decks(pad, bottom + 12.0, w - 2.0 * pad);
        }
        self.content_h = bottom + self.scroll - HDR + 20.0;
    }

    pub(crate) fn load_to_deck(&mut self, d: usize) {
        let lib = self.lib.clone();
        if let Some(q) = self.queue.first() {
            let t = &lib[q.track];
            let audio = t.audio(); // decode before taking the audio lock
            let cached = if self.use_stems { self.stem_cache.iter().find(|c| c.0 == q.track).map(|c| c.1.clone()) } else { None };
            let mixer = self.mixer.clone();
            let mut m = mixer.lock().unwrap();
            let dk = &mut m.decks[d];
            // Same song already here: keep its layers (no flicker); otherwise take them from the cache.
            if dk.track != Some(q.track) || dk.stems.is_none() {
                dk.stems = cached;
            }
            dk.pcm = Some(audio);
            dk.track = Some(q.track);
            dk.pos = t.a.origin;
            dk.rate = 1.0;
            dk.target_rate = 1.0;
            dk.bpm = t.a.bpm;
            dk.origin = t.a.origin;
            dk.norm = t.a.gain;
            dk.playing = false;
            dk.to_main = true;
            dk.to_dj = false;
        }
    }

    fn deck_action(&mut self, d: usize, k: usize) {
        let mixer = self.mixer.clone();
        let mut m = mixer.lock().unwrap();
        let q = m.quantize;
        match k {
            0 => {
                let start = !m.decks[d].playing;
                m.decks[d].playing = start;
                // Snap to beat: a deck started while the other plays comes in on its beat.
                if start && q {
                    m.snap_phase(d, false);
                }
            }
            1 => {
                if let Some(c) = m.decks[d].cues[0] {
                    m.decks[d].pos = c;
                    if q && m.decks[d].playing {
                        m.snap_phase(d, false);
                    }
                } else {
                    let o = m.decks[d].origin;
                    let spb = m.decks[d].spb();
                    let p = m.decks[d].pos;
                    // A cue set with Snap on sits exactly on the nearest beat.
                    m.decks[d].cues[0] = Some(if q { o + ((p - o) / spb).round() * spb } else { p });
                }
            }
            2 => {
                if m.decks[d].loop_on {
                    m.decks[d].loop_on = false;
                } else {
                    let spb = m.decks[d].spb();
                    let o = m.decks[d].origin;
                    let a = o + ((m.decks[d].pos - o) / spb).floor().max(0.0) * spb;
                    let sel = self.deck_sel[d].filter(|s| m.decks[d].track == Some(s.0));
                    if let Some((_, sa, sb)) = sel {
                        m.decks[d].loop_a = sa;
                        m.decks[d].loop_b = sb;
                        if m.decks[d].pos < sa || m.decks[d].pos >= sb {
                            m.decks[d].pos = sa;
                            m.decks[d].need_reset = true;
                        }
                    } else {
                        m.decks[d].loop_a = a;
                        m.decks[d].loop_b = a + 4.0 * spb;
                    }
                    m.decks[d].loop_on = true;
                }
            }
            3 => {
                if m.decks[d].sync {
                    m.decks[d].sync = false;
                    drop(m);
                    self.say("Beat sync off for this deck");
                } else if m.sync_deck(d) {
                    drop(m);
                    self.say("Beat sync on: tempo and beats locked to the other deck");
                } else {
                    // Other deck silent: match its tempo at least.
                    let target = m.decks[1 - d].eff_bpm();
                    let r = (target / m.decks[d].bpm) as f64;
                    m.decks[d].rate = r.clamp(0.85, 1.15);
                    m.decks[d].target_rate = m.decks[d].rate;
                    m.decks[d].ease = false;
                    drop(m);
                    self.say("Tempo matched - play the other deck to lock the beats too");
                }
            }
            4 => {
                m.decks[d].sync = false;
                m.decks[d].rate = (m.decks[d].rate - 0.005).max(0.8);
                m.decks[d].target_rate = m.decks[d].rate;
                m.decks[d].ease = false;
            }
            5 => {
                m.decks[d].sync = false;
                m.decks[d].rate = (m.decks[d].rate + 0.005).min(1.2);
                m.decks[d].target_rate = m.decks[d].rate;
                m.decks[d].ease = false;
            }
            _ => {
                // DJ only: play this deck in the headphones, never to the audience.
                let on = !(m.decks[d].to_dj && !m.decks[d].to_main);
                let ok = m.set_dj_only(d, on);
                let sel = self.deck_sel[d].filter(|s| m.decks[d].track == Some(s.0));
                let mut listening = false;
                if ok && on {
                    // a chosen part: loop just that part in the headphones, from its start
                    if let Some((_, a, b)) = sel {
                        let dk = &mut m.decks[d];
                        dk.loop_a = a;
                        dk.loop_b = b;
                        dk.loop_on = true;
                        dk.pos = a;
                        dk.need_reset = true;
                        dk.playing = true;
                        listening = true;
                    }
                } else if ok && !on && sel.is_some() {
                    m.decks[d].loop_on = false;
                }
                drop(m);
                if ok {
                    self.say(if listening { "DJ only: the part you chose loops in your headphones, the audience cannot hear it" } else if on { "DJ only: this deck plays in your headphones, the audience cannot hear it. Tip: drag on the wave to pick a part first" } else { "Back to the audience output" });
                } else if self.snap.active == d && self.snap.playing[d] {
                    self.say("That deck is on air - use the other deck for DJ only");
                } else {
                    self.out_menu = true; // show where to pick the headphones
                    self.say("Pick your headphones under DJ (right column), then press DJ only again");
                }
            }
        }
    }


    fn pro_decks(&mut self, x: f32, y: f32, w: f32) -> f32 {
        let lib = self.lib.clone();
        let mixer = self.mixer.clone();
        let narrow = w < 900.0;
        let pw = if narrow { w } else { (w - 200.0) / 2.0 };
        let manual = self.snap.manual;
        let mut ends = y;
        for d in 0..2usize {
            let (px, py) = if narrow {
                (x, y + d as f32 * 346.0)
            } else if d == 0 {
                (x, y)
            } else {
                (x + pw + 200.0, y)
            };
            rrect(px, py, pw, 330.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.06));
            let name = if d == 0 { "DECK A" } else { "DECK B" };
            let on_air = self.snap.active == d;
            let tid = self.snap.track[d];
            let accent = tid.map(|i| hsv(warm_hue(lib[i].hue), 0.55, 1.0)).unwrap_or(WHITE);
            self.mark("decks");
            draw_text(name, px + 16.0, py + 26.0, 18.0, if on_air { accent } else { Color::new(1.0, 1.0, 1.0, 0.5) });
            if on_air {
                draw_text("ON AIR", px + 90.0, py + 26.0, 14.0, Color::new(1.0, 0.4, 0.4, 1.0));
            }
            if let Some(i) = tid {
                let t = &lib[i];
                let eff = t.a.bpm * self.snap.rate[d] as f32;
                draw_text(&fit(&format!("{} - {}", t.title, t.artist), 20.0, pw - 32.0), px + 16.0, py + 52.0, 20.0, WHITE);
                draw_text(
                    &format!("{:.2} BPM ({:+.1}%)   {}   Energy {:.0}%", eff, (self.snap.rate[d] - 1.0) * 100.0, camelot_str(t.a.key, t.a.minor), t.energy * 100.0),
                    px + 16.0,
                    py + 74.0,
                    16.0,
                    Color::new(1.0, 1.0, 1.0, 0.7),
                );
                let marks = vec![(t.a.origin, Color::new(0.3, 1.0, 0.6, 0.8)), (t.a.outro_start, Color::new(1.0, 0.7, 0.2, 0.9))];
                self.wave(px + 16.0, py + 88.0, pw - 32.0, 56.0, t, Some(self.snap.pos[d]), &marks, accent);
                self.deck_wave_select(d, i, px + 16.0, py + 88.0, pw - 32.0, 56.0, accent);
            } else {
                draw_text("Empty", px + 16.0, py + 52.0, 20.0, Color::new(1.0, 1.0, 1.0, 0.4));
            }
            let by = py + 156.0;
            let bw = (pw - 32.0 - 6.0 * 6.0) / 7.0;
            let labels = [if self.snap.playing[d] { "Pause" } else { "Play" }, "Cue", if self.snap.loop_on[d] { "Loop off" } else { "Loop 4" }, if self.snap.sync[d] { "Synced" } else { "Sync" }, "BPM -", "BPM +", if self.snap.dj_only[d] { "DJ only ON" } else { "DJ only" }];
            for (k, l) in labels.iter().enumerate() {
                let active = (k == 0 && self.snap.playing[d]) || (k == 2 && self.snap.loop_on[d]) || (k == 3 && self.snap.sync[d]) || (k == 6 && self.snap.dj_only[d]);
                if self.button(px + 16.0 + k as f32 * (bw + 6.0), by, bw, 32.0, l, active, accent) {
                    self.deck_action(d, k);
                }
            }
            // EQ and filter as rotary knobs (drag up/down, right-click resets).
            let ky = by + 100.0;
            let kr = 26.0;
            let cell = (pw - 32.0) / 4.0;
            let names = ["Bass", "Mids", "Treble"];
            for k in 0..4 {
                let cx = px + 16.0 + cell * (k as f32 + 0.5);
                if k < 3 {
                    let v = self.snap.eq[d][k] / 1.5;
                    let db = 20.0 * (self.snap.eq[d][k].max(0.001)).log10();
                    let txt = if self.snap.eq[d][k] < 0.02 { "kill".to_string() } else { format!("{:+.1} dB", db) };
                    if let Some(nv) = self.knob(cx, ky, kr, v, 1.0 / 1.5, names[k], &txt, accent) {
                        mixer.lock().unwrap().decks[d].eq[k] = nv * 1.5;
                    }
                } else {
                    let v = self.snap.filt[d];
                    let txt = if v > 0.99 { "open".to_string() } else { format!("{:.0}%", v * 100.0) };
                    if let Some(nv) = self.knob(cx, ky, kr, v, 1.0, "Filter", &txt, accent) {
                        mixer.lock().unwrap().decks[d].filt = nv;
                    }
                }
            }
            if manual && self.button(px + pw - 130.0, py + 8.0, 114.0, 24.0, "Load next", false, accent) {
                self.load_to_deck(d);
            }
            ends = ends.max(py + 340.0);
        }
        let cx = if narrow { x } else { x + pw + 12.0 };
        let cy0 = if narrow { y + 2.0 * 346.0 } else { y };
        let cw = if narrow { w } else { 176.0 };
        rrect(cx, cy0, cw, 330.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.06));
        draw_text("MIXER", cx + 16.0, cy0 + 26.0, 16.0, Color::new(1.0, 1.0, 1.0, 0.6));
        let a = Color::new(0.5, 0.9, 1.0, 1.0);
        // Auto mix: finds the best spot in the next ~10 s and runs a planned transition.
        if self.button(cx + cw - 16.0 - 84.0, cy0 + 8.0, 84.0, 26.0, "Auto mix", self.snap.trans.is_some(), a) {
            self.auto_mix_now();
        }
        let deck_col = |d: usize| self.snap.track[d].map(|i| hsv(warm_hue(lib[i].hue), 0.55, 1.0)).unwrap_or(a);
        let (ca, cb) = (deck_col(0), deck_col(1));
        // meters read in dB: -36 dB .. 0 dB
        let db_level = |l: f32| (1.0 + 20.0 * l.max(1e-5).log10() / 36.0).clamp(0.0, 1.0);
        let (lx, rx) = (cx + 30.0, cx + cw - 30.0);
        let mid = cx + cw / 2.0;
        // Row 1: TRIM A, MASTER, TRIM B
        let ky = cy0 + 60.0;
        for d in 0..2 {
            let kx = if d == 0 { lx } else { rx };
            let t = self.snap.trim[d];
            let txt = format!("{:+.1} dB", 20.0 * t.max(0.001).log10());
            if let Some(nv) = self.knob(kx, ky, 15.0, t / 2.0, 0.5, "TRIM", &txt, if d == 0 { ca } else { cb }) {
                mixer.lock().unwrap().decks[d].trim = nv * 2.0;
            }
        }
        let mv = self.snap.master;
        if let Some(nv) = self.knob(mid, ky, 15.0, mv, 0.8, "MASTER", &format!("{:.0}%", mv * 100.0), a) {
            mixer.lock().unwrap().master = nv;
        }
        // Row 2: channel faders with level meters, master meter in the middle
        let fy = cy0 + 112.0;
        let fh = 104.0;
        for d in 0..2 {
            let fx = if d == 0 { lx } else { rx };
            let col = if d == 0 { ca } else { cb };
            if let Some(nv) = self.vfader(fx + 4.0, fy, fh, self.snap.fader[d], 1.0, col) {
                mixer.lock().unwrap().decks[d].fader = nv;
            }
            let mx_ = if d == 0 { fx + 22.0 } else { fx - 30.0 };
            self.meter(mx_, fy, 6.0, fh, db_level(self.snap.level[d]));
            draw_text(if d == 0 { "A" } else { "B" }, fx, fy + fh + 18.0, 14.0, col);
        }
        let peak = self.snap.scope.iter().fold(0f32, |m, v| m.max(v.abs()));
        self.meter(mid - 8.0, fy, 6.0, fh, db_level(peak));
        self.meter(mid + 2.0, fy, 6.0, fh, db_level(peak));
        draw_text("M", mid - 5.0, fy + fh + 18.0, 13.0, Color::new(1.0, 1.0, 1.0, 0.5));
        // Row 3: crossfader and its curve
        let xy = cy0 + 254.0;
        let mut v = self.snap.xf;
        if self.slider(cx + 16.0, xy, cw - 32.0, &mut v, a) && manual {
            mixer.lock().unwrap().xfader = v;
        }
        let cur = self.snap.xf_curve;
        let hw = (cw - 38.0) / 2.0;
        if self.button(cx + 16.0, xy + 14.0, hw, 22.0, "Smooth", cur < 0.5, a) {
            mixer.lock().unwrap().xf_curve = 0.0;
        }
        if self.button(cx + 22.0 + hw, xy + 14.0, hw, 22.0, "Cut", cur >= 0.5, a) {
            mixer.lock().unwrap().xf_curve = 1.0;
        }
        // Row 4: manual / mix now
        let by = cy0 + 296.0;
        if self.button(cx + 16.0, by, hw, 26.0, if manual { "Manual" } else { "Auto" }, manual, a) {
            let mut m = mixer.lock().unwrap();
            m.manual = !m.manual;
            m.pending = None;
            drop(m);
            self.armed = None;
        }
        self.tip(cx + 22.0 + hw, by, hw, 26.0, "M");
        if self.button(cx + 22.0 + hw, by, hw, 26.0, "Mix now", false, a) {
            self.skip();
        }
        ends.max(cy0 + 340.0)
    }

    pub fn queue_screen(&mut self, w: f32) {
        let lib = self.lib.clone();
        let accent = self.acc;
        let mut y = HDR + 18.0 - self.scroll;
        let x = 32.0;
        let rw = w - 64.0;
        self.tx("queue", "Up Next", x, y + 30.0, 36.0, WHITE);
        let bw = 150.0;
        if self.button(w - 32.0 - bw, y + 4.0, bw, 34.0, "Smart sort", false, accent) {
            smart_sort(&lib, Some(self.current), &mut self.queue, &self.set, &self.prof);
            self.dirty = true;
        }
        if self.button(w - 32.0 - 2.0 * bw - 8.0, y + 4.0, bw, 34.0, "DJ refill", false, accent) {
            self.rebuild_queue();
        }
        y += 58.0;
        draw_text("The DJ keeps this queue full. Pin tracks to lock them, or move and remove anything.", x, y, 17.0, Color::new(1.0, 1.0, 1.0, 0.55));
        y += 26.0;
        let mut action: Option<(usize, usize)> = None;
        let n = self.queue.len();
        for i in 0..n {
            let (track, pinned, manual, reason) = {
                let q = &self.queue[i];
                (q.track, q.pinned, q.manual, q.reason.clone())
            };
            let t = &lib[track];
            let hh = 86.0;
            rrect(x, y, rw, hh - 10.0, 12.0, Color::new(1.0, 1.0, 1.0, if pinned { 0.12 } else { 0.06 }));
            art(x + 12.0, y + 11.0, 52.0, t);
            let tx = x + 78.0;
            let btn_w = if w < 760.0 { 0.0 } else { 86.0 + 56.0 + 34.0 * 3.0 + 70.0 + 30.0 };
            let mw = rw - 94.0 - btn_w;
            draw_text(&fit(&format!("{}. {}", i + 1, t.title), 22.0, mw), tx, y + 28.0, 22.0, WHITE);
            let info = format!("{}  |  {:.0} BPM  |  {}  |  E{:.0}%{}", t.artist, t.a.bpm, camelot_str(t.a.key, t.a.minor), t.energy * 100.0, if manual { "  |  added by you" } else { "" });
            draw_text(&fit(&info, 16.0, mw), tx, y + 49.0, 16.0, Color::new(1.0, 1.0, 1.0, 0.65));
            let why = reason.replace(" | ", "; ");
            draw_text(&fit(&why, 14.0, mw), tx, y + 66.0, 14.0, Color::new(accent.r, accent.g, accent.b, 0.8));
            if btn_w > 0.0 {
                let mut cx = x + rw - btn_w - 6.0;
                let labels = ["Mix now", "Next", "^", "v", if pinned { "Pinned" } else { "Pin" }, "x"];
                let ws = [86.0, 56.0, 34.0, 34.0, 70.0, 34.0];
                for (k, l) in labels.iter().enumerate() {
                    if self.button(cx, y + 23.0, ws[k], 30.0, l, k == 4 && pinned, accent) {
                        action = Some((i, k));
                    }
                    cx += ws[k] + 6.0;
                }
            }
            y += hh;
        }
        self.content_h = y + self.scroll - HDR + 24.0;
        if let Some((i, k)) = action {
            match k {
                0 => {
                    let t = self.queue[i].track;
                    self.play_now(t);
                }
                1 if i > 0 => {
                    let it = self.queue.remove(i);
                    self.queue.insert(0, it);
                    self.say("Moved to next - the DJ mixes into it after this song");
                }
                2 if i > 0 => self.queue.swap(i, i - 1),
                3 if i + 1 < self.queue.len() => self.queue.swap(i, i + 1),
                4 => self.queue[i].pinned = !self.queue[i].pinned,
                5 => {
                    self.queue.remove(i);
                    self.refill();
                }
                _ => {}
            }
            self.dirty = true;
        }
    }

    pub fn library_screen(&mut self, w: f32) {
        let lib = self.lib.clone();
        let accent = self.acc;
        let mut y = HDR + 18.0 - self.scroll;
        let x = 32.0;
        let rw = w - 64.0;
        self.tx("library", "Library", x, y + 30.0, 36.0, WHITE);
        y += 58.0;
        // add-your-music panel + search
        let count_mine = lib.iter().filter(|t| t.user).count();
        let scanning = self.scanning();
        let panel_h = if scanning { 158.0 } else if self.scan_failed.is_empty() { 140.0 } else { 140.0 + 20.0 * self.scan_failed.len().min(3) as f32 };
        rrect(x, y, rw, panel_h, 16.0, Color::new(1.0, 1.0, 1.0, 0.06));
        rrect(x, y, 5.0, panel_h, 2.0, accent);
        draw_text("YOUR MUSIC", x + 24.0, y + 28.0, 16.0, accent);
        let headline = if scanning {
            "Scanning your folder in the background...".to_string()
        } else if count_mine == 0 {
            "Bring your own songs".to_string()
        } else {
            format!("{} of your songs are ready to mix", count_mine)
        };
        draw_text(&fit(&headline, 26.0, rw - 48.0), x + 24.0, y + 58.0, 26.0, WHITE);
        if scanning {
            let (phase, cur, done, found) = {
                let p = self.loader.as_ref().unwrap().lock().unwrap();
                (p.phase.clone(), p.current.clone(), p.done, p.found)
            };
            let frac = if found > 0 { done as f32 / found as f32 } else { 0.0 };
            rrect(x + 24.0, y + 74.0, rw - 48.0, 8.0, 4.0, Color::new(1.0, 1.0, 1.0, 0.15));
            if frac > 0.0 {
                rrect(x + 24.0, y + 74.0, ((rw - 48.0) * frac).max(8.0), 8.0, 4.0, accent);
            } else {
                let t = (self.time * 0.8).fract();
                rrect(x + 24.0 + (rw - 48.0 - 120.0) * t, y + 74.0, 120.0, 8.0, 4.0, accent);
            }
            let line = if found > 0 { format!("{}: {} of {} songs  -  {}", phase, done, found, cur) } else { phase };
            draw_text(&fit(&line, 17.0, rw - 48.0), x + 24.0, y + 108.0, 17.0, Color::new(1.0, 1.0, 1.0, 0.75));
            draw_text("You can keep listening and browsing while this runs.", x + 24.0, y + 132.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.5));
        } else {
            let msg = if !self.scan_msg.is_empty() { self.scan_msg.clone() } else { "Choose any folder with MP3, WAV, FLAC or OGG files. Each song is analysed on this computer for tempo, key and energy.".to_string() };
            draw_text(&fit(&msg, 17.0, rw - 48.0), x + 24.0, y + 84.0, 17.0, Color::new(1.0, 1.0, 1.0, 0.75));
            draw_text(&fit(&format!("Folder: {}", self.music_dir), 14.0, rw - 520.0), x + 24.0, y + 112.0, 14.0, Color::new(1.0, 1.0, 1.0, 0.45));
            let by = y + 98.0;
            if self.button(x + rw - 24.0 - 150.0, by, 150.0, 34.0, "Reload", false, accent) {
                self.begin_scan();
            }
            if self.button(x + rw - 24.0 - 150.0 - 190.0, by, 180.0, 34.0, "Choose folder...", true, accent) {
                if let Some(p) = rfd::FileDialog::new().set_title("Choose the folder with your songs").pick_folder() {
                    crate::tracks::set_music_dir(p);
                    self.begin_scan();
                }
            }
            let demo_label = if self.use_demo { "Demo songs: ON" } else { "Demo songs: OFF" };
            if self.button(x + rw - 24.0 - 150.0 - 190.0 - 190.0, by, 180.0, 34.0, demo_label, self.use_demo, accent) {
                self.use_demo = !self.use_demo;
                self.rebuild_queue();
            }
            let fails = self.scan_failed.clone();
            for (i, f) in fails.iter().take(3).enumerate() {
                draw_text(&fit(&format!("Skipped: {}", f), 14.0, rw - 48.0), x + 24.0, y + 140.0 + 20.0 * i as f32, 14.0, Color::new(1.0, 0.6, 0.5, 0.9));
            }
        }
        y += panel_h + 24.0;
        let sb_w = (rw).min(420.0);
        let focus = self.search_focus;
        let hv = self.hov(x, y, sb_w, 36.0);
        if self.click {
            self.search_focus = hv;
        }
        rrect(x, y, sb_w, 36.0, 18.0, Color::new(1.0, 1.0, 1.0, if focus { 0.16 } else { 0.09 }));
        let shown = if self.search.is_empty() && !focus { "Search songs, artists, genres...".to_string() } else { format!("{}{}", self.search, if focus && (self.time * 2.0) as i32 % 2 == 0 { "|" } else { "" }) };
        draw_text(&fit(&shown, 19.0, sb_w - 28.0), x + 16.0, y + 25.0, 19.0, if self.search.is_empty() { Color::new(1.0, 1.0, 1.0, 0.45) } else { WHITE });
        if focus {
            while let Some(ch) = get_char_pressed() {
                if !ch.is_control() && self.search.len() < 40 {
                    self.search.push(ch);
                }
            }
            if is_key_pressed(KeyCode::Backspace) {
                self.search.pop();
            }
            if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Enter) {
                self.search_focus = false;
            }
        }
        y += 58.0;
        draw_text("MADE FOR YOU - learned from your likes, skips and listening", x, y + 10.0, 16.0, accent);
        y += 28.0;
        let mut scored: Vec<(f32, usize)> = Vec::new();
        let recent: Vec<usize> = self.prof.history.iter().rev().take(6).cloned().collect();
        for t in lib.iter() {
            if t.id == self.current {
                continue;
            }
            let p = score_candidate(t, Some(&lib[self.current]), None, &self.set, &self.prof, &recent);
            scored.push((p.score + 0.6 * self.prof.taste(t), t.id));
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let n = if w < 760.0 { 2 } else { 3 };
        let cw = (rw - (n as f32 - 1.0) * 12.0) / n as f32;
        let mut act: Option<(usize, bool)> = None;
        for k in 0..n.min(scored.len()) {
            let cx = x + k as f32 * (cw + 12.0);
            let t = &lib[scored[k].1];
            rrect(cx, y, cw, 100.0, 12.0, Color::new(1.0, 1.0, 1.0, 0.07));
            art(cx + 10.0, y + 10.0, 76.0, t);
            draw_text(&fit(&t.title, 20.0, cw - 110.0), cx + 98.0, y + 30.0, 20.0, WHITE);
            draw_text(&fit(&t.artist, 15.0, cw - 110.0), cx + 98.0, y + 49.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.6));
            let tag = if self.prof.plays[t.id] == 0 { "New for you" } else { "Back for more" };
            draw_text(tag, cx + 98.0, y + 66.0, 14.0, accent);
            if self.button(cx + 98.0, y + 72.0, 70.0, 22.0, "Queue", false, accent) {
                act = Some((t.id, false));
            }
        }
        y += 132.0;
        draw_text("ALL TRACKS - analysed on this device (BPM, key and energy are measured from the audio)", x, y + 10.0, 16.0, Color::new(1.0, 1.0, 1.0, 0.5));
        y += 28.0;
        for t in lib.iter() {
            let q = self.search.to_lowercase();
            if !q.is_empty() && !format!("{} {} {}", t.title, t.artist, t.genre).to_lowercase().contains(&q) {
                continue;
            }
            rrect(x, y, rw, 58.0, 10.0, Color::new(1.0, 1.0, 1.0, if t.id == self.current { 0.13 } else { 0.05 }));
            art(x + 8.0, y + 7.0, 44.0, t);
            let narrow = w < 900.0;
            let mw = if narrow { rw - 220.0 } else { 300.0 };
            draw_text(&fit(&t.title, 20.0, mw), x + 62.0, y + 26.0, 20.0, WHITE);
            draw_text(&fit(&format!("{}  |  {}", t.artist, t.genre), 15.0, mw), x + 62.0, y + 46.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.6));
            if !narrow {
                let sx = x + 62.0 + 316.0;
                draw_text(&format!("{:.1} BPM", t.a.bpm), sx, y + 26.0, 18.0, accent);
                draw_text(&format!("{}  {}", camelot_str(t.a.key, t.a.minor), key_name(t.a.key, t.a.minor)), sx, y + 46.0, 15.0, WHITE);
                draw_text("Energy", sx + 130.0, y + 22.0, 13.0, Color::new(1.0, 1.0, 1.0, 0.5));
                self.energy_bar(sx + 130.0, y + 28.0, 90.0, t.energy, accent);
                draw_text(&format!("Dance {:.0}%", t.dance * 100.0), sx + 130.0, y + 50.0, 13.0, Color::new(1.0, 1.0, 1.0, 0.6));
                draw_text(&fit(&t.mood, 15.0, 110.0), sx + 240.0, y + 26.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.8));
                draw_text(&format!("Played {}  Skipped {}", self.prof.plays[t.id], self.prof.skips[t.id]), sx + 240.0, y + 46.0, 13.0, Color::new(1.0, 1.0, 1.0, 0.5));
            }
            if self.button(x + rw - 170.0, y + 14.0, 70.0, 30.0, "Play", false, accent) {
                act = Some((t.id, true));
            }
            if self.button(x + rw - 92.0, y + 14.0, 84.0, 30.0, "+ Queue", false, accent) {
                act = Some((t.id, false));
            }
            y += 72.0;
        }
        self.content_h = y + self.scroll - HDR + 24.0;
        if let Some((id, play)) = act {
            if play {
                self.play_now(id);
            } else {
                self.queue.push(QItem { track: id, pinned: false, manual: true, reason: "you added this track".to_string() });
                self.dirty = true;
            }
        }
    }
}

impl App {
    /// Drag on a deck's waveform to choose a part of the song (click clears it). With a part
    /// chosen, "DJ only" loops just that part in the headphones; Loop loops it on the deck.
    fn deck_wave_select(&mut self, d: usize, tid: usize, x: f32, y: f32, w: f32, h: f32, accent: Color) {
        let len = self.lib[tid].samples() as f64;
        let hv = self.hov(x, y, w, h);
        if self.click && hv {
            self.deck_drag = Some((self.mx, d));
        }
        let to = |px: f32| (((px - x) / w).clamp(0.0, 1.0) as f64) * len;
        if let Some((sx, dd)) = self.deck_drag {
            if dd == d {
                let cx = self.mx.clamp(x, x + w);
                if self.down {
                    let (l, r) = (sx.min(cx), sx.max(cx));
                    if r - l > 4.0 {
                        draw_rectangle(l, y, r - l, h, Color::new(1.0, 1.0, 1.0, 0.25));
                    }
                } else {
                    self.deck_drag = None;
                    if (cx - sx).abs() > 6.0 {
                        let (mut a, mut b) = (to(sx.min(cx)), to(sx.max(cx)));
                        let mut m = self.mixer.lock().unwrap();
                        if m.quantize {
                            let o = m.decks[d].origin;
                            let spb = m.decks[d].spb();
                            a = (o + ((a - o) / spb).round() * spb).clamp(0.0, len);
                            b = (o + ((b - o) / spb).round() * spb).clamp(0.0, len);
                        }
                        if b - a < SR as f64 * 0.5 {
                            b = (a + SR as f64 * 0.5).min(len);
                        }
                        // already listening in the headphones: switch to the new part at once
                        let dk = &mut m.decks[d];
                        if dk.to_dj && !dk.to_main && dk.loop_on {
                            dk.loop_a = a;
                            dk.loop_b = b;
                            dk.pos = a;
                            dk.need_reset = true;
                        }
                        drop(m);
                        self.deck_sel[d] = Some((tid, a, b));
                        self.say("Part chosen. Press DJ only to listen to just this part in your headphones (Loop loops it here)");
                    } else {
                        self.deck_sel[d] = None;
                    }
                }
            }
        }
        if let Some((st, a, b)) = self.deck_sel[d] {
            if st == tid {
                let x0 = x + (a / len) as f32 * w;
                let x1 = x + (b / len) as f32 * w;
                draw_rectangle(x0, y, (x1 - x0).max(1.0), h, Color::new(accent.r, accent.g, accent.b, 0.25));
                draw_rectangle(x0, y, 2.0, h, accent);
                draw_rectangle(x1 - 2.0, y, 2.0, h, accent);
                draw_text(&format!("{} - {}", clock_str(a), clock_str(b)), x0 + 5.0, y + 14.0, 14.0, WHITE);
            }
        }
    }

    fn section_overlay(&self, x: f32, y: f32, w: f32, h: f32, tid: usize, c: Color) {
        if let Some(Some((a, b))) = self.sections.get(tid) {
            let len = self.lib[tid].samples() as f64;
            let x0 = x + (*a / len) as f32 * w;
            let x1 = x + (*b / len) as f32 * w;
            // dim everything outside the section
            draw_rectangle(x, y, (x0 - x).max(0.0), h, Color::new(0.0, 0.0, 0.0, 0.55));
            draw_rectangle(x1, y, (x + w - x1).max(0.0), h, Color::new(0.0, 0.0, 0.0, 0.55));
            draw_rectangle(x0, y, (x1 - x0).max(1.0), 3.0, c);
            draw_rectangle(x0, y + h - 3.0, (x1 - x0).max(1.0), 3.0, c);
            draw_text(&format!("{} - {}", clock_str(*a), clock_str(*b)), x0 + 6.0, y + 16.0, 15.0, WHITE);
        }
    }

    /// Drag = choose a section, click on the playing song = jump there,
    /// click on the next song = start it from that point.
    fn wave_interact(&mut self, x: f32, y: f32, w: f32, h: f32, tid: usize, which: u8) {
        let len = self.lib[tid].samples() as f64;
        let hv = self.hov(x, y - 4.0, w, h + 8.0);
        // Ctrl + mouse on the up-next waveform moves only the DJ preview; the audience is untouched.
        if which == 1 && (is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl)) && (hv || self.cue_drag) {
            if self.click && hv {
                self.cue_drag = true;
                let (dj_on, dj_mix) = {
                    let m = self.mixer.lock().unwrap();
                    (!self.dj_name.is_empty(), m.dj_mix)
                };
                if !dj_on {
                    self.say("DJ output is off - open Outputs and pick your headphones first");
                } else if dj_mix {
                    self.say("DJ is set to Main mix - switch Mode to Preview to scrub");
                }
            }
            if self.cue_drag && self.down {
                let p = (((self.mx.clamp(x, x + w) - x) / w) as f64) * len;
                self.mixer.lock().unwrap().cue_seek(tid, p);
                return;
            }
        }
        if self.cue_drag && !self.down {
            self.cue_drag = false;
        }
        if self.cue_drag {
            return;
        }
        if self.click && hv {
            self.drag = Some((self.mx, which));
        }
        if let Some((sx, wh)) = self.drag {
            if wh != which {
                return;
            }
            let cx = self.mx.clamp(x, x + w);
            if self.down {
                let (l, r) = (sx.min(cx), sx.max(cx));
                if r - l > 4.0 {
                    draw_rectangle(l, y, r - l, h, Color::new(1.0, 1.0, 1.0, 0.2));
                }
            } else {
                self.drag = None;
                let to_pos = |px: f32| (((px - x) / w).clamp(0.0, 1.0) as f64) * len;
                if (cx - sx).abs() > 6.0 {
                    self.set_section(tid, to_pos(sx.min(cx)), to_pos(sx.max(cx)));
                } else if which == 0 {
                    let act = self.snap.active;
                    self.jump_deck(act, to_pos(cx));
                } else {
                    let t = &self.lib[tid];
                    let end = (t.a.origin + t.a.last_bar as f64 * 4.0 * SR as f64 * 60.0 / t.a.bpm as f64).min(len);
                    let p = self.snap_bar(tid, to_pos(cx));
                    self.set_section(tid, p, end);
                }
            }
        }
    }

    /// Row of clickable chips, one per transition point. Returns the clicked one.
    fn point_chips(&mut self, x: f32, y: f32, w: f32, label: &str, pts: &[MixPoint], chosen: Option<f64>, tol: f64, accent: Color) -> Option<usize> {
        draw_text(label, x, y + 17.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.5));
        let mut cx = x + 92.0;
        let mut clicked = None;
        for (i, p) in pts.iter().enumerate() {
            let text = format!("{} {}", clock_str(p.pos), p.kind);
            let wd = tw(&text, 18.0) + 30.0;
            if cx + wd > x + w {
                break;
            }
            let on = chosen.map(|c| (c - p.pos).abs() < tol * 0.5).unwrap_or(false);
            if self.button(cx, y, wd, 24.0, &text, on, accent) {
                clicked = Some(i);
            }
            cx += wd + 6.0;
        }
        if pts.is_empty() {
            draw_text("none found (song too short or no clear structure)", cx, y + 17.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.4));
        }
        clicked
    }
}

impl App {
    /// Phase 4: shows the layered transition as one row per layer. In each row the
    /// current song's layer (warm) fades out and the next song's (cool) fades in,
    /// with a playhead while the mix runs. Returns the new y.
    fn layer_timeline(&mut self, sc: &crate::engine::Script, prog: f32, x: f32, y: f32, w: f32, accent: Color) -> f32 {
        let order = [(0usize, "Vocals"), (4, "Melody"), (1, "Music"), (2, "Bass"), (3, "Drums")];
        let row_h = 22.0;
        let h = 52.0 + row_h * order.len() as f32 + 24.0;
        rrect(x, y, w, h, 14.0, Color::new(1.0, 1.0, 1.0, 0.05));
        self.tx("timeline", "LAYER TIMELINE", x + 16.0, y + 26.0, 16.0, accent);
        let warm = Color::new(1.0, 0.62, 0.25, 1.0);
        let cool = Color::new(0.25, 0.8, 1.0, 1.0);
        let lx = x + 90.0;
        let lw = w - 90.0 - 16.0;
        let cells = 64;
        let fade = |win: (f32, f32), t: f32| if win.1 > win.0 { ((t - win.0) / (win.1 - win.0)).clamp(0.0, 1.0) } else if t >= win.0 { 1.0 } else { 0.0 };
        for (r, (k, name)) in order.iter().enumerate() {
            let ry = y + 44.0 + r as f32 * row_h;
            draw_text(name, x + 16.0, ry + 14.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.7));
            rrect(lx, ry + 2.0, lw, row_h - 6.0, 4.0, Color::new(1.0, 1.0, 1.0, 0.06));
            let cw = lw / cells as f32;
            for c in 0..cells {
                let t = (c as f32 + 0.5) / cells as f32;
                let go = (fade(sc[0][*k], t) * std::f32::consts::FRAC_PI_2).cos();
                let gi = (fade(sc[1][*k], t) * std::f32::consts::FRAC_PI_2).sin();
                let half = (row_h - 6.0) / 2.0;
                let cx = lx + c as f32 * cw;
                draw_rectangle(cx, ry + 2.0, cw + 0.5, half, Color::new(warm.r, warm.g, warm.b, 0.15 + 0.75 * go));
                draw_rectangle(cx, ry + 2.0 + half, cw + 0.5, half, Color::new(cool.r, cool.g, cool.b, 0.15 + 0.75 * gi));
            }
        }
        let rows_end = y + 44.0 + row_h * order.len() as f32;
        if prog > 0.0 {
            let px = lx + lw * prog.clamp(0.0, 1.0);
            draw_rectangle(px - 1.0, y + 40.0, 2.0, rows_end - y - 36.0, WHITE);
        }
        let ly = rows_end + 18.0;
        draw_rectangle(lx, ly - 9.0, 12.0, 8.0, warm);
        draw_text("current song", lx + 18.0, ly, 14.0, Color::new(1.0, 1.0, 1.0, 0.55));
        draw_rectangle(lx + 120.0, ly - 9.0, 12.0, 8.0, cool);
        draw_text("next song", lx + 138.0, ly, 14.0, Color::new(1.0, 1.0, 1.0, 0.55));
        draw_text("brighter = louder", lx + 230.0, ly, 14.0, Color::new(1.0, 1.0, 1.0, 0.4));
        y + h + 16.0
    }
}
