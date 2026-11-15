//! Drawing helpers, widgets, the Now Playing screen, header/footer and input.
use crate::app::*;
use crate::dj::*;
use crate::tracks::{camelot_str, key_name, Track, SR};
use macroquad::prelude::*;
use std::sync::Arc;

pub const HDR: f32 = 60.0;
pub const FTR: f32 = 100.0;

pub fn mood_hue(m: Mood) -> f32 {
    match m {
        Mood::Chill => 0.55,
        Mood::Party => 0.85,
        Mood::Workout => 0.02,
        Mood::Focus => 0.38,
        Mood::Romantic => 0.93,
        Mood::Discovery => 0.13,
    }
}

pub fn tw(s: &str, size: f32) -> f32 {
    measure_text(s, None, size as u16, 1.0).width
}

pub fn fit(s: &str, size: f32, maxw: f32) -> String {
    if tw(s, size) <= maxw {
        return s.to_string();
    }
    let mut t: String = s.to_string();
    while !t.is_empty() && tw(&format!("{}..", t), size) > maxw {
        t.pop();
    }
    format!("{}..", t)
}

pub fn wrap(s: &str, size: f32, maxw: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for w in s.split_whitespace() {
        let t = if cur.is_empty() { w.to_string() } else { format!("{} {}", cur, w) };
        if tw(&t, size) > maxw && !cur.is_empty() {
            lines.push(cur);
            cur = w.to_string();
        } else {
            cur = t;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

pub fn mmss(secs: f32) -> String {
    let s = secs.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

thread_local! {
    static FONT: std::cell::RefCell<Option<Font>> = std::cell::RefCell::new(None);
    static FONT_BOLD: std::cell::RefCell<Option<Font>> = std::cell::RefCell::new(None);
    static COVERS: std::cell::RefCell<std::collections::HashMap<usize, Texture2D>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Loads a clean system UI font (Segoe UI on Windows); falls back to the built-in font.
pub fn init_fonts() {
    let dirs = [std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into()) + "\\Fonts\\", "/usr/share/fonts/truetype/dejavu/".into()];
    let load = |names: &[&str]| -> Option<Font> {
        for d in &dirs {
            for n in names {
                if let Ok(b) = std::fs::read(format!("{}{}", d, n)) {
                    if let Ok(mut f) = load_ttf_font_from_bytes(&b) {
                        f.set_filter(FilterMode::Linear);
                        return Some(f);
                    }
                }
            }
        }
        None
    };
    let reg = load(&["segoeui.ttf", "DejaVuSans.ttf"]);
    let bold = load(&["segoeuib.ttf", "DejaVuSans-Bold.ttf"]).or_else(|| reg.clone());
    FONT.with(|f| *f.borrow_mut() = reg);
    FONT_BOLD.with(|f| *f.borrow_mut() = bold);
}

fn with_font<R>(size: f32, g: impl FnOnce(Option<&Font>) -> R) -> R {
    // Large text reads better in the bold cut, like streaming-app headlines.
    let cell = if size >= 34.0 { &FONT_BOLD } else { &FONT };
    cell.with(|f| g(f.borrow().as_ref()))
}

/// Drop-in replacement for macroquad's draw_text that uses the UI font.
pub fn draw_text(s: &str, x: f32, y: f32, size: f32, c: Color) -> TextDimensions {
    with_font(size, |f| {
        let fs = (size * 0.86).round() as u16;
        draw_text_ex(s, x.round(), y.round(), TextParams { font: f, font_size: fs, color: c, ..Default::default() })
    })
}

/// Drop-in replacement for macroquad's measure_text that uses the UI font.
pub fn measure_text(s: &str, _f: Option<&Font>, size: u16, scale: f32) -> TextDimensions {
    with_font(size as f32, |f| macroquad::text::measure_text(s, f, (size as f32 * 0.86).round() as u16, scale))
}

/// Rounded rectangle drawn as ONE triangle fan, so translucent fills never overlap.
pub fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32, c: Color) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let r = r.min(h / 2.0).min(w / 2.0).max(0.0);
    let steps = if r > 12.0 { 10 } else { 6 };
    let col: [u8; 4] = [(c.r * 255.0) as u8, (c.g * 255.0) as u8, (c.b * 255.0) as u8, (c.a * 255.0) as u8];
    let v = |px: f32, py: f32| macroquad::models::Vertex { position: vec3(px, py, 0.0), uv: vec2(0.0, 0.0), color: col.into(), normal: vec4(0.0, 0.0, 0.0, 0.0) };
    let mut verts = vec![v(x + w / 2.0, y + h / 2.0)];
    let corners = [(x + w - r, y + h - r, 0.0f32), (x + r, y + h - r, 90.0), (x + r, y + r, 180.0), (x + w - r, y + r, 270.0)];
    for (cx, cy, a0) in corners {
        for i in 0..=steps {
            let a = (a0 + 90.0 * i as f32 / steps as f32).to_radians();
            verts.push(v(cx + r * a.cos(), cy + r * a.sin()));
        }
    }
    let n = verts.len() as u16;
    let mut idx = Vec::with_capacity(n as usize * 3);
    for i in 1..n {
        let j = if i + 1 < n { i + 1 } else { 1 };
        idx.extend_from_slice(&[0, i, j]);
    }
    draw_mesh(&Mesh { vertices: verts, indices: idx, texture: None });
}

/// Frees all cached cover textures (call when the library is replaced).
pub fn clear_covers() {
    COVERS.with(|m| m.borrow_mut().clear());
}

pub fn art(x: f32, y: f32, s: f32, t: &Track) {
    if let Some(cv) = &t.cover {
        let key = Arc::as_ptr(cv) as usize;
        let tex = COVERS.with(|m| {
            m.borrow_mut()
                .entry(key)
                .or_insert_with(|| {
                    let tx = Texture2D::from_rgba8(cv.0, cv.1, &cv.2);
                    tx.set_filter(FilterMode::Linear);
                    tx
                })
                .clone()
        });
        draw_texture_ex(&tex, x, y, WHITE, DrawTextureParams { dest_size: Some(vec2(s, s)), ..Default::default() });
        return;
    }
    art_generated(x, y, s, t);
}

fn art_generated(x: f32, y: f32, s: f32, t: &Track) {
    let c1 = hsv(t.hue / 360.0, 0.65, 0.9);
    let c2 = hsv((t.hue + 60.0) / 360.0, 0.75, 0.30);
    let n = 40;
    for i in 0..n {
        let f = i as f32 / n as f32;
        let c = Color::new(c1.r + (c2.r - c1.r) * f, c1.g + (c2.g - c1.g) * f, c1.b + (c2.b - c1.b) * f, 1.0);
        draw_rectangle(x, y + s * f, s, s / n as f32 + 1.0, c);
    }
    let a = hsv((t.hue + 30.0) / 360.0, 0.4, 1.0);
    match t.id % 3 {
        0 => {
            draw_circle(x + s * 0.65, y + s * 0.38, s * 0.26, Color::new(a.r, a.g, a.b, 0.55));
            draw_circle_lines(x + s * 0.65, y + s * 0.38, s * 0.34, 3.0, Color::new(1.0, 1.0, 1.0, 0.35));
            draw_circle(x + s * 0.28, y + s * 0.72, s * 0.14, Color::new(0.0, 0.0, 0.0, 0.25));
        }
        1 => {
            for k in 0..7 {
                let o = k as f32 * s * 0.13;
                draw_line(x + o, y + s, x + s, y + o, s * 0.04, Color::new(1.0, 1.0, 1.0, 0.18));
            }
            draw_circle(x + s * 0.5, y + s * 0.45, s * 0.2, Color::new(a.r, a.g, a.b, 0.7));
        }
        _ => {
            draw_triangle(vec2(x + s * 0.15, y + s * 0.85), vec2(x + s * 0.85, y + s * 0.85), vec2(x + s * 0.5, y + s * 0.2), Color::new(a.r, a.g, a.b, 0.55));
            draw_circle(x + s * 0.5, y + s * 0.2, s * 0.07, Color::new(1.0, 1.0, 1.0, 0.7));
        }
    }
}

impl App {
    pub fn hov(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        if self.clip && (self.my < HDR || self.my > screen_height() - FTR) {
            return false;
        }
        self.mx >= x && self.mx <= x + w && self.my >= y && self.my <= y + h
    }

    pub fn tx(&mut self, tag: &'static str, s: &str, x: f32, y: f32, size: f32, c: Color) {
        self.mark(tag);
        draw_text(s, x, y, size, c);
    }

    pub fn mark(&mut self, tag: &'static str) {
        if !tag.is_empty() && !self.drawn.contains(&tag) {
            self.drawn.push(tag);
        }
    }

    pub fn button(&mut self, x: f32, y: f32, w: f32, h: f32, label: &str, active: bool, accent: Color) -> bool {
        let hv = self.hov(x, y, w, h);
        // Key by x + label (not y) so the animation survives scrolling.
        let lh = label.bytes().fold(5381i32, |a, b| a.wrapping_mul(33) ^ b as i32);
        let dt = self.dt;
        let ease = |cur: &mut f32, target: f32, speed: f32| *cur += (target - *cur) * (dt * speed).min(1.0);
        let hk = self.anim.entry((x as i32, lh)).or_insert(0.0);
        ease(hk, if hv { 1.0 } else { 0.0 }, 14.0);
        let hvv = *hk;
        let ak = self.anim.entry((x as i32 ^ 0x5a5a, lh)).or_insert(if active { 1.0 } else { 0.0 });
        ease(ak, if active { 1.0 } else { 0.0 }, 12.0);
        let act = *ak;
        // Hover grows the pill slightly, pressing shrinks it, like streaming apps.
        let s = 1.0 + 0.04 * hvv - if hv && self.down { 0.05 } else { 0.0 };
        let (bw, bh) = (w * s, h * s);
        let (bx, by) = (x - (bw - w) / 2.0, y - (bh - h) / 2.0);
        let idle = 0.09 + 0.09 * hvv;
        let mix = |a: f32, b: f32| a + (b - a) * act;
        let base = Color::new(mix(1.0, accent.r), mix(1.0, accent.g), mix(1.0, accent.b), mix(idle, 0.95));
        rrect(bx, by, bw, bh, bh / 2.0, base);
        let size = if h > 30.0 { 22.0 } else { 18.0 };
        let l = fit(label, size, w - 10.0);
        let wd = tw(&l, size);
        let tv = 1.0 - 0.95 * act;
        let tc = Color::new(tv, tv, tv + 0.03 * act, 1.0);
        draw_text(&l, x + (w - wd) / 2.0, y + h / 2.0 + size * 0.3, size, tc);
        hv && self.click
    }

    pub fn slider(&mut self, x: f32, y: f32, w: f32, v: &mut f32, accent: Color) -> bool {
        let hv = self.hov(x - 8.0, y - 12.0, w + 16.0, 24.0);
        let mut changed = false;
        if hv && self.down {
            *v = ((self.mx - x) / w).clamp(0.0, 1.0);
            changed = true;
        }
        rrect(x, y - 2.0, w, 4.0, 2.0, Color::new(1.0, 1.0, 1.0, 0.18));
        rrect(x, y - 2.0, (w * *v).max(4.0), 4.0, 2.0, accent);
        draw_circle(x + w * *v, y, if hv { 8.0 } else { 6.5 }, WHITE);
        changed
    }

    pub fn chip(&mut self, x: f32, y: f32, label: &str, c: Color) -> f32 {
        let wd = tw(label, 17.0) + 18.0;
        rrect(x, y, wd, 24.0, 12.0, Color::new(c.r, c.g, c.b, 0.22));
        draw_text(label, x + 9.0, y + 17.0, 17.0, c);
        wd + 6.0
    }

    pub fn wave(&self, x: f32, y: f32, w: f32, h: f32, t: &Track, playhead: Option<f64>, marks: &[(f64, Color)], tint: Color) {
        rrect(x, y, w, h, 6.0, Color::new(1.0, 1.0, 1.0, 0.05));
        let p = &t.a.peaks;
        let n = ((w / 3.0) as usize).max(8);
        for i in 0..n {
            let a = (i * p.len() / n).min(p.len() - 1);
            let b = ((i + 1) * p.len() / n).max(a + 1).min(p.len());
            let v = p[a..b].iter().fold(0f32, |m, &q| m.max(q));
            let bh = (v * h * 0.9).max(1.0);
            draw_rectangle(x + i as f32 * w / n as f32, y + (h - bh) / 2.0, (w / n as f32 - 1.0).max(1.0), bh, Color::new(tint.r, tint.g, tint.b, 0.75));
        }
        let len = t.samples() as f64;
        for (m, c) in marks {
            let mx = x + (*m / len).clamp(0.0, 1.0) as f32 * w;
            draw_line(mx, y, mx, y + h, 2.0, *c);
        }
        if let Some(ph) = playhead {
            let px = x + (ph / len).clamp(0.0, 1.0) as f32 * w;
            draw_line(px, y - 3.0, px, y + h + 3.0, 2.5, WHITE);
        }
    }

    pub fn energy_bar(&self, x: f32, y: f32, w: f32, v: f32, c: Color) {
        rrect(x, y, w, 6.0, 3.0, Color::new(1.0, 1.0, 1.0, 0.15));
        rrect(x, y, (w * v).max(6.0), 6.0, 3.0, c);
    }

    pub fn draw_visualizer(&self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        let n = 32;
        let bw = w / n as f32;
        for i in 0..n {
            let v = self.vis[i];
            let bh = (v * h).max(3.0);
            draw_rectangle(x + i as f32 * bw + 1.0, y + h - bh, bw - 3.0, bh, Color::new(c.r, c.g, c.b, 0.35 + 0.6 * v));
        }
    }

    pub fn now_playing(&mut self, w: f32) {
        let lib = self.lib.clone();
        let t = &lib[self.current];
        let accent = self.acc;
        let narrow = w < 860.0;
        let a = self.snap.active;
        let spb = SR as f64 * 60.0 / t.a.bpm as f64;
        let phase = ((self.snap.pos[a] - t.a.origin) / spb).rem_euclid(1.0) as f32;
        let pulse = (1.0 - phase).powi(3);
        let s = if narrow { (w - 60.0).min(330.0) } else { (w * 0.36).min(440.0) };
        let (ax, ay) = if narrow { ((w - s) / 2.0, HDR + 16.0 - self.scroll) } else { (50.0, HDR + 30.0 - self.scroll) };
        let ss = s * (1.0 + 0.018 * pulse);
        rrect(ax - 4.0 - (ss - s) / 2.0, ay - 4.0 - (ss - s) / 2.0 + 14.0, ss + 8.0, ss + 8.0, 18.0, Color::new(accent.r, accent.g, accent.b, 0.12));
        art(ax - (ss - s) / 2.0, ay - (ss - s) / 2.0, ss, t);
        let (ix, mut iy) = if narrow { (24.0, ay + s + 50.0) } else { (ax + s + 56.0, ay + 20.0) };
        let iw = if narrow { w - 48.0 } else { w - ix - 40.0 };
        self.tx("genre", &t.genre.to_uppercase(), ix, iy, 20.0, accent);
        iy += if narrow { 40.0 } else { 58.0 };
        let title_size = if narrow { 40.0 } else { 58.0 };
        let title = fit(&t.title, title_size, iw);
        self.tx("title", &title, ix, iy, title_size, WHITE);
        iy += 34.0;
        let sub = format!("{}  -  {}", t.artist, t.album);
        self.tx("artist", &fit(&sub, 26.0, iw), ix, iy, 26.0, Color::new(1.0, 1.0, 1.0, 0.7));
        iy += 22.0;
        let mut cx = ix;
        let eff = t.a.bpm * self.snap.rate[a] as f32;
        cx += self.chip(cx, iy, &format!("{:.1} BPM", eff), accent);
        self.mark("bpm");
        cx += self.chip(cx, iy, &format!("{} ({})", key_name(t.a.key, t.a.minor), camelot_str(t.a.key, t.a.minor)), accent);
        self.mark("key");
        cx += self.chip(cx, iy, &format!("Energy {:.0}%", t.energy * 100.0), accent);
        if cx + 120.0 < ix + iw {
            self.chip(cx, iy, &t.mood, accent);
        }
        iy += 48.0;
        let vh = if narrow { 80.0 } else { 110.0 };
        self.draw_visualizer(ix, iy, iw, vh, accent);
        self.mark("visualizer");
        iy += vh + 22.0;
        let liked = self.prof.liked[t.id];
        let disliked = self.prof.disliked[t.id];
        if self.button(ix, iy, 110.0, 34.0, if liked { "Liked" } else { "Like" }, liked, accent) {
            self.prof.like(t);
        }
        if self.button(ix + 120.0, iy, 120.0, 34.0, if disliked { "Disliked" } else { "Dislike" }, disliked, accent) {
            self.prof.dislike(t);
            if !disliked {
                self.skip();
            }
        }
        let auto = self.set.auto;
        if self.button(ix + 250.0, iy, 150.0, 34.0, if auto { "DJ MODE: ON" } else { "DJ MODE: OFF" }, auto, accent) {
            self.set.auto = !self.set.auto;
            self.dirty = true;
        }
        iy += 54.0;
        self.tx("mood", "LISTENING MODE", ix, iy, 16.0, Color::new(1.0, 1.0, 1.0, 0.5));
        iy += 12.0;
        let mut mx = ix;
        let mut my = iy;
        for m in MOODS {
            let wd = tw(m.name(), 20.0) + 28.0;
            if mx + wd > ix + iw && mx > ix {
                mx = ix;
                my += 42.0;
            }
            let on = self.set.mood == m;
            if self.button(mx, my, wd, 34.0, m.name(), on, hsv(mood_hue(m), 0.5, 1.0)) {
                self.set.mood = m;
                self.rebuild_queue();
            }
            mx += wd + 8.0;
        }
        let bottom = my + 60.0;
        self.content_h = bottom + self.scroll - HDR;
        if !self.queue.is_empty() {
            let q = &self.queue[0];
            let nt = &lib[q.track];
            let y = bottom;
            rrect(ix, y, iw, 56.0, 10.0, Color::new(1.0, 1.0, 1.0, 0.07));
            art(ix + 8.0, y + 8.0, 40.0, nt);
            let line = format!("Up next: {} - {}", nt.title, nt.artist);
            let why = q.reason.split(" | ").next().unwrap_or("").to_string();
            self.tx("queue", &fit(&line, 20.0, iw - 70.0), ix + 58.0, y + 26.0, 20.0, WHITE);
            draw_text(&fit(&why, 15.0, iw - 70.0), ix + 58.0, y + 46.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.55));
            self.content_h += 70.0;
        }
    }

    pub fn header_footer(&mut self, w: f32, h: f32) {
        self.clip = false;
        let lib = self.lib.clone();
        let t = &lib[self.current];
        let accent = self.acc;
        draw_rectangle(0.0, 0.0, w, HDR, Color::new(0.04, 0.04, 0.06, 1.0));
        let narrow = w < 760.0;
        if let Some(tex) = &self.logo {
            // Soft glow that breathes with the music clock.
            let pulse = 0.5 + 0.5 * (self.time * 2.4).sin();
            draw_circle(34.0, 30.0, 21.0 + 2.0 * pulse, Color::new(0.93, 0.28, 0.60, 0.10 + 0.08 * pulse));
            draw_texture_ex(tex, 16.0, 12.0, WHITE, DrawTextureParams { dest_size: Some(vec2(36.0, 36.0)), ..Default::default() });
        }
        if !narrow {
            draw_text("VIBE CRAFT", 62.0, 38.0, 26.0, WHITE);
        }
        if self.loader.is_none() {
            if let Some(st) = self.upd.clone() {
                let s = st.lock().unwrap().clone();
                let bx = if narrow { 60.0 } else { 220.0 };
                use crate::update::State;
                match s {
                    State::Available { tag, .. } => {
                        let label = format!("Update to {}", tag);
                        let bw = tw(&label, 18.0) + 30.0;
                        if self.button(bx, 14.0, bw, 32.0, &label, true, Color::new(0.13, 0.83, 0.93, 1.0)) {
                            crate::update::install(&st);
                        }
                    }
                    State::Downloading { pct, .. } => {
                        let msg = format!("Updating... {:.0}%", pct * 100.0);
                        let wd = tw(&msg, 17.0) + 24.0;
                        rrect(bx, 17.0, wd, 26.0, 13.0, Color::new(0.13, 0.83, 0.93, 0.2));
                        rrect(bx, 17.0, wd * pct.clamp(0.0, 1.0), 26.0, 13.0, Color::new(0.13, 0.83, 0.93, 0.35));
                        draw_text(&msg, bx + 12.0, 35.0, 17.0, WHITE);
                    }
                    State::Ready { tag } => {
                        let label = format!("Restart for {}", tag);
                        let bw = tw(&label, 18.0) + 30.0;
                        if self.button(bx, 14.0, bw, 32.0, &label, true, Color::new(0.3, 1.0, 0.6, 1.0)) {
                            crate::update::restart();
                        }
                    }
                    State::Failed(e) => {
                        // Only show download/install failures; a failed check stays quiet.
                        if e.starts_with("install:") {
                            draw_text(&e, bx, 35.0, 16.0, Color::new(1.0, 0.5, 0.5, 0.9));
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(l) = &self.loader {
            let (d, f) = {
                let p = l.lock().unwrap();
                (p.done, p.found)
            };
            let msg = if f > 0 { format!("Scanning music {}/{}", d, f) } else { "Scanning music...".to_string() };
            let wd = tw(&msg, 17.0) + 34.0;
            let bx = if narrow { 60.0 } else { 220.0 };
            rrect(bx, 17.0, wd, 26.0, 13.0, Color::new(accent.r, accent.g, accent.b, 0.2));
            let pulse = 0.5 + 0.5 * (self.time * 5.0).sin();
            draw_circle(bx + 14.0, 30.0, 4.0, Color::new(accent.r, accent.g, accent.b, 0.4 + 0.6 * pulse));
            draw_text(&msg, bx + 24.0, 35.0, 17.0, WHITE);
        }
        let names: [&str; 4] = if narrow { ["Now", "DJ", "Queue", "Library"] } else { ["Now Playing", "Smart DJ", "Queue", "Library"] };
        let mut x = w - 16.0;
        for i in (0..4).rev() {
            let wd = tw(names[i], 20.0) + 30.0;
            x -= wd;
            if self.button(x, 12.0, wd - 6.0, 36.0, names[i], self.tab == i, accent) {
                self.tab = i;
                self.scroll = 0.0;
            }
        }
        let fy = h - FTR;
        draw_rectangle(0.0, fy, w, FTR, Color::new(0.04, 0.04, 0.06, 1.0));
        let ver = concat!("v", env!("CARGO_PKG_VERSION"));
        draw_text(ver, w - tw(ver, 13.0) - 10.0, h - 8.0, 13.0, Color::new(1.0, 1.0, 1.0, 0.4));
        let a = self.snap.active;
        let len = self.snap.len[a].max(1.0);
        let pos_frac = (self.snap.pos[a] / len).clamp(0.0, 1.0) as f32;
        let px = 24.0;
        let pw = w - 48.0;
        rrect(px, fy + 10.0, pw, 6.0, 3.0, Color::new(1.0, 1.0, 1.0, 0.18));
        rrect(px, fy + 10.0, (pw * pos_frac).max(6.0), 6.0, 3.0, accent);
        self.mark("progress");
        if let Some(ar) = &self.armed {
            let m = (ar.trigger / len).clamp(0.0, 1.0) as f32;
            draw_rectangle(px + pw * m - 1.0, fy + 6.0, 3.0, 14.0, Color::new(0.3, 1.0, 0.6, 0.9));
        }
        if self.click && self.mx >= px && self.mx <= px + pw && self.my >= fy && self.my <= fy + 26.0 {
            let f = ((self.mx - px) / pw) as f64;
            let mixer = self.mixer.clone();
            let mut m = mixer.lock().unwrap();
            let act = m.active;
            let l = m.decks[act].len();
            m.decks[act].pos = f * l;
            drop(m);
            self.dirty = true;
        }
        let secs = (self.snap.pos[a] / SR as f64) as f32;
        draw_text(&mmss(secs), px, fy + 36.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.6));
        let tot = mmss((len / SR as f64) as f32);
        draw_text(&tot, px + pw - tw(&tot, 15.0), fy + 36.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.6));
        art(24.0, fy + 42.0, 48.0, t);
        if !narrow {
            draw_text(&fit(&t.title, 20.0, w * 0.25), 82.0, fy + 62.0, 20.0, WHITE);
            draw_text(&fit(&t.artist, 15.0, w * 0.25), 82.0, fy + 82.0, 15.0, Color::new(1.0, 1.0, 1.0, 0.6));
        }
        let cx = w / 2.0;
        let cy = fy + 66.0;
        let h_prev = self.hov(cx - 102.0, cy - 22.0, 44.0, 44.0);
        let c = if h_prev { WHITE } else { Color::new(1.0, 1.0, 1.0, 0.8) };
        draw_triangle(vec2(cx - 70.0, cy - 10.0), vec2(cx - 70.0, cy + 10.0), vec2(cx - 86.0, cy), c);
        draw_rectangle(cx - 90.0, cy - 10.0, 3.0, 20.0, c);
        if h_prev && self.click {
            self.prev();
        }
        let ph = self.hov(cx - 22.0, cy - 22.0, 44.0, 44.0);
        draw_circle(cx, cy, if ph { 25.0 } else { 23.0 }, WHITE);
        if self.snap.paused {
            draw_triangle(vec2(cx - 7.0, cy - 11.0), vec2(cx - 7.0, cy + 11.0), vec2(cx + 12.0, cy), BLACK);
        } else {
            draw_rectangle(cx - 9.0, cy - 11.0, 6.0, 22.0, BLACK);
            draw_rectangle(cx + 3.0, cy - 11.0, 6.0, 22.0, BLACK);
        }
        if ph && self.click {
            self.toggle_pause();
        }
        let h_next = self.hov(cx + 58.0, cy - 22.0, 44.0, 44.0);
        let c = if h_next { WHITE } else { Color::new(1.0, 1.0, 1.0, 0.8) };
        draw_triangle(vec2(cx + 70.0, cy - 10.0), vec2(cx + 70.0, cy + 10.0), vec2(cx + 86.0, cy), c);
        draw_rectangle(cx + 87.0, cy - 10.0, 3.0, 20.0, c);
        if h_next && self.click {
            self.skip();
        }
        let auto = self.set.auto;
        let bw = if narrow { 70.0 } else { 120.0 };
        let lbl = match (auto, narrow) {
            (true, true) => "DJ",
            (true, false) => "AUTO DJ",
            (false, true) => "dj",
            (false, false) => "DJ off",
        };
        if self.button(w - 24.0 - bw, cy - 17.0, bw, 34.0, lbl, auto, accent) {
            self.set.auto = !self.set.auto;
            self.dirty = true;
        }
        if !narrow {
            let mut v = self.snap.master;
            draw_text("Vol", w - 24.0 - bw - 170.0, cy + 5.0, 16.0, Color::new(1.0, 1.0, 1.0, 0.6));
            if self.slider(w - 24.0 - bw - 140.0, cy, 110.0, &mut v, accent) {
                self.mixer.lock().unwrap().master = v;
            }
        }
        self.menu_rect = (0.0, 0.0, 0.0, 0.0);
        if !self.out_devices.is_empty() {
            let dj_on = !self.dj_name.is_empty();
            let dw = if narrow { 96.0 } else { 120.0 };
            let dx = if narrow { w - 24.0 - bw - 8.0 - dw } else { w - 24.0 - bw - 180.0 - dw };
            let by = cy - 15.0;
            let label = if dj_on { "Outputs + DJ" } else { "Outputs" };
            if self.button(dx, by, dw, 30.0, label, self.out_menu || dj_on, accent) {
                self.out_menu = !self.out_menu;
            }
            if self.out_menu {
                let lim = 10usize;
                let devs: Vec<String> = self.out_devices.iter().take(lim).cloned().collect();
                let (dj_mix, dj_vol) = {
                    let m = self.mixer.lock().unwrap();
                    (m.dj_mix, m.dj_vol)
                };
                let rh = 26.0f32;
                let mw = (w - 16.0).min(640.0);
                let colw = (mw - 24.0) / 2.0;
                let mx = (dx + dw - mw).max(8.0).min(w - mw - 8.0).max(4.0);
                let nrows = devs.len() + 2 + 1; // header + default/off + devices (+ DJ has one more)
                let list_h = rh * (nrows + 1) as f32;
                let total = list_h + rh * 5.0 + 8.0;
                let my = (fy - 8.0 - total).max(HDR + 4.0);
                rrect(mx - 4.0, my - 4.0, mw + 8.0, total + 8.0, 10.0, Color::new(0.08, 0.08, 0.1, 0.98));
                self.menu_rect = (mx - 4.0, my - 4.0, mw + 8.0, total + 8.0);
                let dim = Color::new(1.0, 1.0, 1.0, 0.45);
                let mut pick_main: Option<Option<String>> = None;
                let mut pick_dj: Option<String> = None;
                // column 0 = audience, column 1 = DJ
                for col in 0..2 {
                    let cx = mx + 8.0 + col as f32 * (colw + 8.0);
                    draw_text(if col == 0 { "AUDIENCE (speakers, main mix)" } else { "DJ (headphones)" }, cx, my + 18.0, 14.0, dim);
                    let mut items: Vec<(String, String)> = Vec::new();
                    if col == 1 {
                        items.push(("Off".into(), String::new()));
                    }
                    items.push(("System default".into(), "*".into()));
                    for n in &devs {
                        items.push((n.clone(), n.clone()));
                    }
                    for (i, (lbl, val)) in items.iter().enumerate() {
                        let ry = my + 26.0 + rh * i as f32;
                        let hv = self.hov(cx, ry, colw, rh);
                        let cur = if col == 0 {
                            val != "*" && *val == self.out_name
                        } else if val.is_empty() {
                            self.dj_name.is_empty()
                        } else {
                            val != "*" && *val == self.dj_name
                        };
                        if hv {
                            rrect(cx, ry, colw, rh, 6.0, Color::new(1.0, 1.0, 1.0, 0.1));
                        }
                        if cur {
                            draw_circle(cx + 7.0, ry + rh / 2.0, 3.5, accent);
                        }
                        draw_text(&fit(lbl, 15.0, colw - 24.0), cx + 18.0, ry + 18.0, 15.0, if cur { accent } else { WHITE });
                        if hv && self.click {
                            if col == 0 {
                                pick_main = Some(if val == "*" { None } else { Some(val.clone()) });
                            } else {
                                pick_dj = Some(val.clone());
                            }
                        }
                    }
                }
                // DJ options
                let oy = my + list_h;
                draw_line(mx + 8.0, oy, mx + mw - 8.0, oy, 1.0, Color::new(1.0, 1.0, 1.0, 0.12));
                let hears = if !dj_on {
                    "DJ output is off - pick a device on the right".to_string()
                } else if dj_mix {
                    "DJ hears the main mix (same as the audience)".to_string()
                } else {
                    let a = self.snap.active;
                    match self.snap.track[1 - a] {
                        Some(i) => format!("DJ hears a private preview of: {}", self.lib[i].title),
                        None => "DJ preview: waiting for the next song to load".to_string(),
                    }
                };
                draw_text(&fit(&hears, 15.0, mw - 20.0), mx + 10.0, oy + 19.0, 15.0, if dj_on { accent } else { dim });
                let bw2 = (mw - 16.0 - 12.0) / 3.0;
                let by2 = oy + rh;
                if self.button(mx + 8.0, by2, bw2, 24.0, if dj_mix { "Mode: Main mix" } else { "Mode: Preview" }, false, accent) {
                    let mut m = self.mixer.lock().unwrap();
                    m.dj_mix = !m.dj_mix;
                    m.cue_restart();
                }
                if self.button(mx + 8.0 + bw2 + 6.0, by2, bw2, 24.0, "Restart preview", false, accent) {
                    self.mixer.lock().unwrap().cue_restart();
                }
                if self.button(mx + 8.0 + 2.0 * (bw2 + 6.0), by2, bw2, 24.0, "Close", false, accent) {
                    self.out_menu = false;
                }
                draw_text("Audience volume", mx + 10.0, by2 + rh + 19.0, 15.0, WHITE);
                let mut av = self.snap.master;
                if self.slider(mx + 150.0, by2 + rh + 14.0, mw - 170.0, &mut av, accent) {
                    self.mixer.lock().unwrap().master = av;
                }
                draw_text("DJ volume", mx + 10.0, by2 + 2.0 * rh + 19.0, 15.0, WHITE);
                let mut v = dj_vol / 1.5;
                if self.slider(mx + 150.0, by2 + 2.0 * rh + 14.0, mw - 170.0, &mut v, accent) {
                    self.mixer.lock().unwrap().dj_vol = v * 1.5;
                }
                if let Some(p) = pick_main {
                    self.out_request = Some(p);
                } else if let Some(p) = pick_dj {
                    self.dj_request = Some(p);
                } else if self.click && !self.hov(dx, by, dw, 30.0) && !self.hov(mx - 4.0, my - 4.0, mw + 8.0, total + 8.0) {
                    self.out_menu = false;
                }
            }
        }
        if self.toast_t > 0.0 {
            let tw_ = tw(&self.toast, 20.0) + 30.0;
            rrect((w - tw_) / 2.0, HDR + 8.0, tw_, 34.0, 17.0, Color::new(0.0, 0.0, 0.0, 0.8));
            draw_text(&self.toast, (w - tw_) / 2.0 + 15.0, HDR + 31.0, 20.0, WHITE);
        }
        self.clip = true;
    }

    pub fn draw(&mut self) {
        let (w, h) = (screen_width(), screen_height());
        let n = 40;
        for i in 0..n {
            let f = i as f32 / n as f32;
            let k = (1.0 - f).powf(1.5);
            let c = Color::new(0.03 + self.amb.r * k, 0.03 + self.amb.g * k, 0.045 + self.amb.b * k, 1.0);
            draw_rectangle(0.0, h * f, w, h / n as f32 + 1.0, c);
        }
        self.mx = mouse_position().0;
        self.my = mouse_position().1;
        self.click = is_mouse_button_pressed(MouseButton::Left);
        self.down = is_mouse_button_down(MouseButton::Left);
        let (real_click, real_down) = (self.click, self.down);
        let (rx0, ry0, rw0, rh0) = self.menu_rect;
        let under_menu = self.out_menu && rw0 > 0.0 && self.mx >= rx0 && self.mx <= rx0 + rw0 && self.my >= ry0 && self.my <= ry0 + rh0;
        if under_menu {
            self.click = false;
            self.down = false;
        }
        let wy = mouse_wheel().1;
        let (zx, zy, zw, zh) = self.zoom_rect;
        let over_zoom = self.tab == 1 && self.zoom && zw > 0.0 && self.mx >= zx && self.mx <= zx + zw && self.my >= zy && self.my <= zy + zh;
        self.zoom_rect = (0.0, 0.0, 0.0, 0.0);
        if wy != 0.0 && over_zoom {
            // Wheel over the beat zoom: zoom in (up) or out (down).
            self.zoom_secs = (self.zoom_secs * if wy > 0.0 { 0.8 } else { 1.25 }).clamp(0.5, 32.0);
        } else if wy != 0.0 && !under_menu {
            self.scroll -= wy.signum() * 50.0;
        }
        let max_scroll = (self.content_h - (h - HDR - FTR)).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max_scroll);
        match self.tab {
            0 => self.now_playing(w),
            1 => self.smart_dj(w),
            2 => self.queue_screen(w),
            _ => self.library_screen(w),
        }
        self.click = real_click;
        self.down = real_down;
        self.header_footer(w, h);
        if self.tab == 3 && self.search_focus {
            return;
        }
        if is_key_pressed(KeyCode::Space) {
            self.toggle_pause();
        }
        if is_key_pressed(KeyCode::Right) {
            self.skip();
        }
        if is_key_pressed(KeyCode::Left) {
            self.prev();
        }
        if is_key_pressed(KeyCode::M) {
            self.skip();
            self.say("Mixing into the next track");
        }
        if is_key_pressed(KeyCode::D) {
            self.set.auto = !self.set.auto;
            self.dirty = true;
        }
        for (i, k) in [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4].iter().enumerate() {
            if is_key_pressed(*k) {
                self.tab = i;
                self.scroll = 0.0;
            }
        }
        if is_key_pressed(KeyCode::Up) || is_key_pressed(KeyCode::Down) {
            let d = if is_key_pressed(KeyCode::Up) { 0.05 } else { -0.05 };
            let mut m = self.mixer.lock().unwrap();
            m.master = (m.master + d).clamp(0.0, 1.0);
        }
        if is_key_pressed(KeyCode::L) {
            let t = self.lib[self.current].clone();
            self.prof.like(&t);
        }
    }
}
