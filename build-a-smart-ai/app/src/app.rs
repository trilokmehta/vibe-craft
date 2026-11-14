//! Application state, DJ scheduling logic and session control.
use crate::dj::*;
use crate::engine::*;
use crate::stems::Stems;
use crate::stemctl::StemJob;
use crate::tracks::{Track, SR};
use macroquad::prelude::*;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

pub fn hsv(h: f32, s: f32, v: f32) -> Color {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i as i32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color::new(r, g, b, 1.0)
}

pub fn goertzel(seg: &[f32], freq: f32, sr: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * freq / sr;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0f32, 0f32);
    for &x in seg {
        let s0 = x + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0).sqrt() / seg.len().max(1) as f32
}

#[derive(Clone, Default)]
pub struct Snap {
    pub active: usize,
    pub pos: [f64; 2],
    pub rate: [f64; 2],
    pub track: [Option<usize>; 2],
    pub playing: [bool; 2],
    pub len: [f64; 2],
    pub trans: Option<(f32, Style, usize, usize)>,
    pub paused: bool,
    pub master: f32,
    pub xf: f32,
    pub level: [f32; 2],
    pub eq: [[f32; 3]; 2],
    pub filt: [f32; 2],
    pub manual: bool,
    pub loop_on: [bool; 2],
    pub scope: Vec<f32>,
    pub stems: [bool; 2],
    pub sg: [[f32; 4]; 2],
    pub sr: f32,
    pub completed: u32,
}

#[derive(Clone)]
pub struct Armed {
    pub track: usize,
    pub trigger: f64,
    pub style: Style,
    pub beats: u32,
    pub in_rate: f64,
    pub out_eff: f32,
    pub ks: f32,
    pub matched: bool,
    pub l_trk: f64,
    pub cur: usize,
    pub in_pos: f64,
    pub why: String,
}

fn next_boundary(origin: f64, step: f64, min: f64) -> f64 {
    let k = ((min - origin) / step).ceil().max(0.0);
    origin + k * step
}

/// When set, AutoMix plans the way it did before the energy-aware planner (for A/B tests).
pub static CLASSIC_AUTOMIX: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub struct App {
    pub lib: Rc<Vec<Track>>,
    pub mixer: Arc<Mutex<Mixer>>,
    pub set: Settings,
    pub prof: Profile,
    pub queue: Vec<QItem>,
    pub current: usize,
    pub tab: usize,
    pub listened: f32,
    pub seen_completed: u32,
    pub armed: Option<Armed>,
    pub dirty: bool,
    pub advanced: bool,
    pub scroll: f32,
    pub content_h: f32,
    pub vis: [f32; 32],
    pub drawn: Vec<&'static str>,
    pub skip_flag: bool,
    pub snap: Snap,
    pub mx: f32,
    pub my: f32,
    pub click: bool,
    pub down: bool,
    pub clip: bool,
    pub amb: Color,
    /// Accent colour, eased toward the current song so track changes glide.
    pub acc: Color,
    /// Per-widget hover animation (0..1), keyed by position.
    pub anim: std::collections::HashMap<(i32, i32), f32>,
    pub dt: f32,
    pub last_save: f32,
    pub persist: bool,
    pub genres: Vec<String>,
    pub toast: String,
    pub toast_t: f32,
    pub time: f32,
    pub stem_cache: Vec<(usize, Arc<Stems>)>,
    pub stem_job: Option<(usize, Arc<StemJob>)>,
    pub use_stems: bool,
    pub sections: Vec<Option<(f64, f64)>>,
    pub forced_out: Option<(usize, f64)>,
    pub drag: Option<(f32, u8)>,
    pub search: String,
    pub loader: Option<std::sync::Arc<Mutex<crate::tracks::Progress>>>,
    pub use_demo: bool,
    pub scan_msg: String,
    pub scan_failed: Vec<String>,
    pub search_focus: bool,
    pub music_dir: String,
    /// Output devices available, the one in use, and a pending switch request.
    pub out_devices: Vec<String>,
    pub out_name: String,
    pub out_request: Option<Option<String>>,
    pub out_menu: bool,
    /// DJ (headphone) output: current device name ("" = off) and a pending change
    /// ("" = off, "*" = system default, otherwise a device name).
    pub dj_name: String,
    pub dj_request: Option<String>,
    /// Last drawn output-menu rectangle (x, y, w, h); clicks inside it must not reach the page behind.
    pub menu_rect: (f32, f32, f32, f32),
    pub cue_drag: bool,
}

impl App {
    pub fn new(lib: Rc<Vec<Track>>, mixer: Arc<Mutex<Mixer>>, persist: bool) -> App {
        let mut prof = Profile::new(lib.len());
        if persist {
            prof.load(&lib);
        }
        let mut genres: Vec<String> = Vec::new();
        for t in lib.iter() {
            if !genres.contains(&t.genre) {
                genres.push(t.genre.clone());
            }
        }
        let mut app = App {
            lib,
            mixer,
            set: Settings::new(),
            prof,
            queue: Vec::new(),
            current: 0,
            tab: 0,
            listened: 0.0,
            seen_completed: 0,
            armed: None,
            dirty: false,
            advanced: false,
            scroll: 0.0,
            content_h: 0.0,
            vis: [0.0; 32],
            drawn: Vec::new(),
            skip_flag: false,
            snap: Snap::default(),
            mx: 0.0,
            my: 0.0,
            click: false,
            down: false,
            clip: true,
            amb: Color::new(0.1, 0.1, 0.15, 1.0),
            acc: Color::new(0.6, 0.95, 0.85, 1.0),
            anim: std::collections::HashMap::new(),
            dt: 0.016,
            last_save: 0.0,
            persist,
            genres,
            out_devices: Vec::new(),
            out_name: String::new(),
            out_request: None,
            out_menu: false,
            dj_name: String::new(),
            dj_request: None,
            menu_rect: (0.0, 0.0, 0.0, 0.0),
            cue_drag: false,
            toast: String::new(),
            toast_t: 0.0,
            time: 0.0,
            stem_cache: Vec::new(),
            stem_job: None,
            use_stems: true,
            sections: Vec::new(),
            forced_out: None,
            drag: None,
            search: String::new(),
            loader: None,
            use_demo: true,
            scan_msg: String::new(),
            scan_failed: Vec::new(),
            search_focus: false,
            music_dir: crate::tracks::primary_music_dir().to_string_lossy().to_string(),
        };
        app.start_session();
        app
    }

    fn start_session(&mut self) {
        let lib = self.lib.clone();
        let ex0 = self.demo_exclusions();
        let first = pick_next(&lib, None, None, &self.set, &self.prof, &ex0, &[]).map(|p| p.0).unwrap_or(0);
        self.current = first;
        {
            let audio = lib[first].audio(); // decode before taking the audio lock
            let mixer = self.mixer.clone();
            let mut m = mixer.lock().unwrap();
            let t = &lib[first];
            let d = &mut m.decks[0];
            d.pcm = Some(audio);
            d.track = Some(first);
            d.pos = t.a.origin;
            d.rate = 1.0;
            d.target_rate = 1.0;
            d.bpm = t.a.bpm;
            d.origin = t.a.origin;
            d.norm = t.a.gain;
            d.playing = true;
            m.active = 0;
        }
        self.snapshot();
        self.queue.clear();
        self.refill();
    }

    pub fn refill(&mut self) {
        let lib = self.lib.clone();
        while self.queue.len() < 4 {
            let chain = self.queue.last().map(|q| q.track).unwrap_or(self.current);
            let mut exclude: Vec<usize> = self.queue.iter().map(|q| q.track).collect();
            exclude.extend(self.demo_exclusions());
            exclude.push(self.current);
            let mut recent: Vec<usize> = self.prof.history.iter().rev().take(6).cloned().collect();
            recent.reverse();
            recent.extend(self.queue.iter().map(|q| q.track));
            recent.push(self.current);
            let eff = if self.queue.is_empty() { Some(self.snap_eff()) } else { None };
            match pick_next(&lib, Some(chain), eff, &self.set, &self.prof, &exclude, &recent) {
                Some((id, why)) => self.queue.push(QItem { track: id, pinned: false, manual: false, reason: why }),
                None => break,
            }
        }
    }

    pub fn snap_eff(&self) -> f32 {
        let a = self.snap.active;
        self.lib[self.current].a.bpm * self.snap.rate[a].max(0.5) as f32
    }

    pub fn rebuild_queue(&mut self) {
        self.queue.retain(|q| q.pinned || q.manual);
        self.refill();
        self.dirty = true;
    }

    pub fn say(&mut self, s: &str) {
        self.toast = s.to_string();
        self.toast_t = 3.0;
    }

    fn cancel_pending(&mut self) {
        let mixer = self.mixer.clone();
        let mut m = mixer.lock().unwrap();
        if m.trans.is_none() {
            m.pending = None;
        }
        self.armed = None;
    }

    /// AutoMix: decides how and where the next queued song will be mixed in.
    /// It looks for the best exit point in the playing song and the best entry
    /// point in the next one (anywhere in either song), matches tempo, picks a
    /// transition style, loads the idle deck and arms the sample-accurate scheduler.
    pub fn plan(&mut self, force: bool) {
        if self.queue.is_empty() {
            return;
        }
        let lib = self.lib.clone();
        let nt = self.queue[0].track;
        let mixer = self.mixer.clone();
        let (act, pos, rate, len, busy, cur, manual) = {
            let m = mixer.lock().unwrap();
            let a = m.active;
            (a, m.decks[a].pos, m.decks[a].rate, m.decks[a].len(), m.trans.is_some(), m.decks[a].track, m.manual)
        };
        if busy || manual {
            return;
        }
        let cur = match cur {
            Some(c) => c,
            None => return,
        };
        let ta = &lib[cur];
        let tin = &lib[nt];
        let out_eff = ta.a.bpm * rate as f32;
        let (r, _mult, ts) = tempo_match(out_eff, tin);
        let auto = self.set.auto;
        let grid_ok = ta.a.beat_conf >= 1.5 && tin.a.beat_conf >= 1.5;
        let matched = auto && grid_ok && ts > 0.0;
        let ks = key_score(ta, tin);
        let sr = SR as f64;
        let spb = sr * 60.0 / ta.a.bpm as f64;
        let spb_in = sr * 60.0 / tin.a.bpm as f64;
        let origin = ta.a.origin;
        let mut a_end = (origin + ta.a.last_bar as f64 * 4.0 * spb).min(len).max(pos + sr);
        let sec_cur = self.sections.get(cur).copied().flatten();
        let sec_next = self.sections.get(nt).copied().flatten();
        if let Some((_, b)) = sec_cur {
            a_end = a_end.min(b.max(pos + 2.0 * sr));
        }
        // Finish before the outgoing song starts fading, so the music never sags into dead air.
        let classic = CLASSIC_AUTOMIX.load(std::sync::atomic::Ordering::Relaxed);
        let a_fade = if classic { f64::MAX } else { loud_end(ta).max(pos + 2.0 * sr) };
        a_end = a_end.min(a_fade);
        let in_default = match sec_next {
            Some((a, _)) => a,
            None => tin.a.origin + tin.a.first_bar as f64 * 4.0 * spb_in,
        };
        let intro_quiet = quiet_intro_secs(tin, in_default);
        let out_limit = if classic { sec_cur.map(|s| s.1) } else { Some(sec_cur.map(|s| s.1).unwrap_or(f64::MAX).min(a_fade)) };
        let opts = MixOpts { force_out: self.forced_out.filter(|f| f.0 == cur && f.1 > pos).map(|f| f.1), force_in: sec_next.map(|s| s.0), out_limit };
        let calm = self.set.mood.calm();
        let mut bi: u32 = if self.set.intensity < 0.34 {
            32
        } else if self.set.intensity < 0.67 {
            16
        } else {
            8
        };
        if calm {
            bi = bi.max(16);
        }
        if force {
            bi = bi.min(8);
        }
        // Tempos too far apart to beat-match: a long, smooth crossfade (like AutoMix), not an echo cut.
        let (style, beats, in_rate) = if !auto || !grid_ok || (!matched && !classic) {
            (Style::Fade, 0u32, 1.0)
        } else if !matched {
            (Style::EchoOut, 8, 1.0)
        } else if ks < 0.45 {
            (Style::EchoOut, 8, r)
        } else if calm || self.set.intensity < 0.7 {
            (Style::Blend, bi, r)
        } else {
            (Style::FilterEcho, bi.min(16), r)
        };
        let dur = if style == Style::Fade {
            if force {
                3.0
            } else {
                // a quiet intro plays underneath the outgoing song, so it arrives at full energy
                if classic { 6.0 } else { (intro_quiet + 3.0).clamp(8.0, 16.0) }
            }
        } else {
            beats as f32 * 60.0 / out_eff.max(40.0)
        };
        let l_trk = if style == Style::Fade { dur as f64 * sr * rate } else { beats as f64 * spb };
        let (trigger, in_pos, why) = if force {
            let t = if style != Style::Fade {
                let b = next_boundary(origin, 4.0 * spb, pos + 0.35 * sr * rate);
                if b + l_trk > a_end {
                    pos
                } else {
                    b
                }
            } else {
                pos
            };
            (t, in_default, format!("you asked for the next song: mixing in on the next bar line, entering {} from its start", tin.title))
        } else if style == Style::Fade {
            let why = if !auto {
                "Auto DJ is off: simple crossfade at the end".to_string()
            } else if !grid_ok {
                format!("no reliable beat grid in one of the songs: {:.0} s smart crossfade before {} fades out", dur, ta.title)
            } else {
                format!("tempos too far apart to beat-match: {:.0} s smart crossfade before {} fades out", dur, ta.title)
            };
            ((a_end - l_trk - 0.2 * sr).max(pos), in_default, why)
        } else if let Some(c) = choose_mix(ta, tin, pos, rate, beats, calm, opts) {
            (c.out_pos, c.in_pos, c.why)
        } else {
            let earliest = pos + 3.0 * sr * rate;
            let desired = (ta.a.outro_start - spb).max(earliest);
            let mut p = next_boundary(origin, 16.0 * spb, desired);
            if p + l_trk > a_end {
                let room = a_end - l_trk;
                let q = origin + ((room - origin) / (4.0 * spb)).floor() * 4.0 * spb;
                p = if q >= earliest { q } else { room.max(pos) };
            }
            (p, in_default, "mixing at the end of the song".to_string())
        };
        let tin_audio = tin.audio(); // decode before taking the audio lock
        // Lock the beats where the songs actually overlap: line up the drum hits there.
        let in_pos = if !classic && matches!(style, Style::Blend | Style::FilterEcho) {
            let a_audio = ta.audio();
            align_entry(&a_audio, trigger, rate, &tin_audio, in_pos, in_rate, 60.0 / out_eff.max(40.0) as f64, (dur as f64).min(10.0))
        } else {
            in_pos
        };
        {
            let mut m = mixer.lock().unwrap();
            let inn = 1 - act;
            let d = &mut m.decks[inn];
            d.pcm = Some(tin_audio);
            d.track = Some(nt);
            d.pos = in_pos;
            d.rate = in_rate;
            d.target_rate = in_rate;
            d.bpm = tin.a.bpm;
            d.origin = tin.a.origin;
            d.norm = tin.a.gain;
            d.playing = false;
            d.eq = [1.0; 3];
            d.filt = 1.0;
            d.loop_on = false;
            d.cues = [None; 4];
            d.ease = false;
            d.stems = None;
            d.sg = [1.0; 4];
            m.pending = Some(Pending { deck_in: inn, trigger, start_pos: in_pos, in_rate, dur_secs: dur, style, out_bpm_eff: out_eff });
        }
        let why = if intro_quiet >= 2.0 { format!("{}; {} has a {:.0} s quiet intro", why, tin.title, intro_quiet) } else { why };
        self.armed = Some(Armed { track: nt, trigger, style, beats, in_rate, out_eff, ks, matched, l_trk, cur, in_pos, why });
    }

    pub fn skip(&mut self) {
        if self.snap.trans.is_some() {
            self.say("Already mixing - the next song is on its way");
            return;
        }
        if self.queue.is_empty() {
            self.refill();
        }
        if self.queue.is_empty() {
            self.say("No next song in the queue yet");
            return;
        }
        if self.snap.manual {
            // In manual (crossfader) mode the DJ does not mix by itself: just load the next song.
            let idle = 1 - self.snap.active;
            self.load_to_deck(idle);
            self.say("Manual mode: next song loaded on the other deck - use the crossfader, or switch manual off");
            return;
        }
        if self.snap.paused {
            self.toggle_pause(); // the mix can only start while the sound is running
        }
        self.skip_flag = true;
        self.plan(true);
        let armed = self.armed.as_ref().map(|a| self.lib[a.track].title.clone());
        match armed {
            Some(t) => self.say(&format!("Mixing into {}", t)),
            None => {
                self.skip_flag = false;
                self.say("Could not start the mix - try again");
            }
        }
    }

    pub fn prev(&mut self) {
        if self.snap.trans.is_some() {
            return;
        }
        let a = self.snap.active;
        if self.listened > 4.0 || self.prof.history.is_empty() {
            let mixer = self.mixer.clone();
            let mut m = mixer.lock().unwrap();
            let o = m.decks[a].origin;
            m.decks[a].pos = o;
            drop(m);
            self.listened = 0.0;
            self.dirty = true;
            return;
        }
        if let Some(t) = self.prof.history.pop() {
            self.queue.insert(0, QItem { track: t, pinned: false, manual: true, reason: "you went back to this track".to_string() });
            self.plan(true);
        }
    }

    pub fn play_now(&mut self, t: usize) {
        if self.snap.trans.is_some() || t == self.current {
            return;
        }
        let lib = self.lib.clone();
        self.prof.replay(&lib[t]);
        self.queue.retain(|q| q.track != t);
        self.queue.insert(0, QItem { track: t, pinned: false, manual: true, reason: "you picked this track".to_string() });
        self.plan(true);
    }

    pub fn toggle_pause(&mut self) {
        let mixer = self.mixer.clone();
        let mut m = mixer.lock().unwrap();
        let a = m.active;
        // A deck can stop on its own (deck button, end of a section) while master isn't
        // paused. Treat that as paused so Play always brings the sound back.
        let silent = !m.decks[a].playing && m.trans.is_none();
        if m.paused || silent {
            m.paused = false;
            if m.decks[a].pcm.is_some() && m.decks[a].pos < m.decks[a].len() {
                m.decks[a].playing = true;
            }
        } else {
            m.paused = true;
        }
    }

    fn on_done(&mut self) {
        let lib = self.lib.clone();
        let newt = {
            let m = self.mixer.lock().unwrap();
            m.decks[m.active].track
        };
        let newt = match newt {
            Some(n) => n,
            None => return,
        };
        let old = self.current;
        let frac = (self.listened / lib[old].dur.max(1.0)).min(1.0);
        let skipped = self.skip_flag;
        self.prof.on_finished(&lib[old], frac, skipped);
        self.skip_flag = false;
        self.current = newt;
        self.listened = 0.0;
        if let Some(i) = self.queue.iter().position(|q| q.track == newt) {
            self.queue.remove(i);
        }
        self.armed = None;
        self.refill();
        self.dirty = false;
        if self.persist {
            self.prof.save(&lib);
        }
    }

    pub fn snapshot(&mut self) {
        let m = self.mixer.lock().unwrap();
        let mut s = Snap::default();
        s.active = m.active;
        for d in 0..2 {
            s.pos[d] = m.decks[d].pos;
            s.rate[d] = m.decks[d].rate;
            s.track[d] = m.decks[d].track;
            s.playing[d] = m.decks[d].playing;
            s.len[d] = m.decks[d].len();
            s.level[d] = m.decks[d].level;
            s.eq[d] = m.decks[d].eq;
            s.filt[d] = m.decks[d].filt;
            s.loop_on[d] = m.decks[d].loop_on;
            s.stems[d] = m.decks[d].stems.is_some();
            s.sg[d] = m.decks[d].sg;
        }
        s.trans = m.trans.map(|t| (t.elapsed as f32 / t.total as f32, t.style, t.out, t.inn));
        s.paused = m.paused || (!m.decks[m.active].playing && m.trans.is_none());
        s.master = m.master;
        s.xf = m.xfader;
        s.manual = m.manual;
        s.sr = m.sr;
        s.completed = m.completed;
        let n = m.scope.len();
        let mut sc = Vec::with_capacity(512);
        for i in 0..512 {
            sc.push(m.scope[(m.scope_head + n - 512 + i) % n]);
        }
        s.scope = sc;
        drop(m);
        self.snap = s;
    }

    pub fn update(&mut self, dt: f32, audio_secs: Option<f32>) {
        self.time += dt;
        self.poll_scan();
        // decode the next songs in the background so they start instantly
        for q in self.queue.iter().take(2) {
            self.lib[q.track].prefetch();
        }
        if let Some(secs) = audio_secs {
            let mixer = self.mixer.clone();
            let mut m = mixer.lock().unwrap();
            if !m.paused {
                m.render_virtual(secs);
            }
        }
        self.snapshot();
        self.manage_stems();
        if self.snap.completed > self.seen_completed {
            self.seen_completed = self.snap.completed;
            self.on_done();
            self.snapshot();
        }
        if !self.snap.paused {
            self.listened += audio_secs.unwrap_or(dt);
        }
        if self.snap.trans.is_none() && !self.queue.is_empty() && !self.snap.manual {
            let cur_eff = self.snap_eff();
            let stale = match &self.armed {
                None => true,
                Some(a) => a.track != self.queue[0].track || (a.out_eff - cur_eff).abs() > 0.05 || self.dirty || a.cur != self.current,
            };
            if stale {
                self.cancel_pending();
                self.dirty = false;
                self.plan(false);
            }
        }
        if self.snap.manual {
            self.armed = None;
        }
        if self.persist && self.time - self.last_save > 30.0 {
            self.last_save = self.time;
            let lib = self.lib.clone();
            self.prof.save(&lib);
        }
        if self.toast_t > 0.0 {
            self.toast_t -= dt;
        }
        let t = &self.lib[self.current];
        let target = hsv(t.hue / 360.0, 0.6, 0.30);
        let k = (dt * 2.0).min(1.0);
        self.amb = Color::new(self.amb.r + (target.r - self.amb.r) * k, self.amb.g + (target.g - self.amb.g) * k, self.amb.b + (target.b - self.amb.b) * k, 1.0);
        let ta = hsv(t.hue / 360.0, 0.45, 1.0);
        let ka = (dt * 3.0).min(1.0);
        self.acc = Color::new(self.acc.r + (ta.r - self.acc.r) * ka, self.acc.g + (ta.g - self.acc.g) * ka, self.acc.b + (ta.b - self.acc.b) * ka, 1.0);
        self.dt = dt;
        let sr = self.snap.sr.max(8000.0);
        for i in 0..32 {
            let f = 60.0 * (10000.0f32 / 60.0).powf(i as f32 / 31.0);
            let v = (goertzel(&self.snap.scope, f, sr) * 10.0).sqrt().min(1.0);
            self.vis[i] = (self.vis[i] * 0.85).max(v);
        }
    }
}

impl App {
    /// Demo tracks are skipped by the DJ once the person has loaded their own songs and turned demos off.
    pub fn demo_exclusions(&self) -> Vec<usize> {
        let users = self.lib.iter().filter(|t| t.user).count();
        if self.use_demo || users < 2 {
            return Vec::new();
        }
        self.lib.iter().filter(|t| !t.user).map(|t| t.id).collect()
    }

    pub fn scanning(&self) -> bool {
        self.loader.is_some()
    }

    /// Starts reading the chosen music folder in the background; playback and the UI keep running.
    pub fn begin_scan(&mut self) {
        if self.loader.is_some() {
            return;
        }
        let dir = crate::tracks::primary_music_dir();
        self.music_dir = dir.to_string_lossy().to_string();
        let prog = std::sync::Arc::new(Mutex::new(crate::tracks::Progress::default()));
        crate::tracks::start_scan(dir, prog.clone());
        self.loader = Some(prog);
        self.scan_msg.clear();
        self.scan_failed.clear();
    }

    pub fn poll_scan(&mut self) {
        let prog = match &self.loader {
            Some(p) => p.clone(),
            None => return,
        };
        let (result, found, failed) = {
            let mut p = prog.lock().unwrap();
            if !p.finished {
                return;
            }
            (p.result.take(), p.found, p.failed.clone())
        };
        self.loader = None;
        let user_tracks = result.unwrap_or_default();
        let loaded = user_tracks.len();
        self.scan_msg = if found == 0 {
            "No MP3, WAV, FLAC or OGG files were found in that folder (subfolders are searched too).".to_string()
        } else if failed.is_empty() {
            format!("Loaded all {} songs from your folder.", loaded)
        } else {
            format!("Loaded {} of {} songs; {} could not be read.", loaded, found, failed.len())
        };
        self.scan_failed = failed;
        let mut merged: Vec<Track> = self.lib.iter().filter(|t| !t.user).cloned().collect();
        let n_demo = merged.len();
        merged.extend(user_tracks);
        crate::tracks::finalize(&mut merged);
        let restart = self.current >= n_demo;
        self.lib = Rc::new(merged);
        self.sections.clear();
        self.stem_cache.clear();
        self.stem_job = None;
        self.forced_out = None;
        let lib = self.lib.clone();
        self.prof = Profile::new(lib.len());
        if self.persist {
            self.prof.load(&lib);
        }
        let users = lib.len() - n_demo;
        if users >= 2 {
            self.use_demo = false;
        }
        let mut genres: Vec<String> = Vec::new();
        for t in lib.iter() {
            if !genres.contains(&t.genre) {
                genres.push(t.genre.clone());
            }
        }
        self.genres = genres;
        self.set.genre = None;
        if restart {
            self.mixer.lock().unwrap().reset();
            self.armed = None;
            self.start_session();
        } else {
            self.queue.clear();
            self.armed = None;
            self.refill();
            self.dirty = true;
        }
    }
}

impl App {
    pub fn snap_bar(&self, tid: usize, pos: f64) -> f64 {
        let t = &self.lib[tid];
        let len = t.samples() as f64;
        if t.a.beat_conf < 1.5 {
            return pos.clamp(0.0, len);
        }
        let bar = 4.0 * SR as f64 * 60.0 / t.a.bpm as f64;
        let p = t.a.origin + ((pos - t.a.origin) / bar).round() * bar;
        p.clamp(0.0, len)
    }

    /// Restricts a song to one section: it is entered at the start of the
    /// section and the DJ mixes out at its end.
    pub fn set_section(&mut self, tid: usize, a: f64, b: f64) {
        if self.sections.len() < self.lib.len() {
            self.sections.resize(self.lib.len(), None);
        }
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let a = self.snap_bar(tid, a);
        let mut b = self.snap_bar(tid, b);
        let min = 2.0 * SR as f64 * 60.0 / self.lib[tid].a.bpm as f64 * 4.0;
        if b - a < min {
            b = (a + min).min(self.lib[tid].samples() as f64);
        }
        self.sections[tid] = Some((a, b));
        if tid == self.current {
            let mixer = self.mixer.clone();
            let mut m = mixer.lock().unwrap();
            let act = m.active;
            let p = m.decks[act].pos;
            if p < a || p > b {
                m.decks[act].pos = a;
            }
        }
        self.dirty = true;
        let name = self.lib[tid].title.clone();
        self.say(&format!("Only the selected section of {} will play", name));
    }

    pub fn clear_section(&mut self, tid: usize) {
        if let Some(s) = self.sections.get_mut(tid) {
            *s = None;
        }
        self.dirty = true;
    }
}
