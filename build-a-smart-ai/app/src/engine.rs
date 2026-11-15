//! Audio playback + mixing engine: two decks with tempo (rate) control,
//! 3-band EQ, filter, echo bus, crossfader, and a sample-accurate transition
//! scheduler. Rendering is device independent so it can also run offline.
use crate::stems::{Stems, NL};
use crate::tracks::SR;
use std::f32::consts::PI;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Style {
    Blend,
    FilterEcho,
    EchoOut,
    Fade,
    /// The original plain equal-power crossfade (kept as a baseline for `--mix-eval`).
    Plain,
}

impl Style {
    pub fn name(&self) -> &'static str {
        match self {
            Style::Blend => "Beat-matched blend + bass swap",
            Style::FilterEcho => "Filter sweep + echo",
            Style::EchoOut => "Echo out, cut in on the one",
            Style::Fade => "Smart crossfade: bass swap, vocals hand off",
            Style::Plain => "Plain crossfade",
        }
    }
}

/// Per-layer fade windows over transition progress 0..1: [outgoing, incoming][layer] = (start, end).
/// Outgoing layers fade out over their window, incoming layers fade in over theirs.
pub type Script = [[(f32, f32); NL]; 2];

#[derive(Clone, Copy)]
pub struct Pending {
    pub deck_in: usize,
    pub trigger: f64,
    pub start_pos: f64,
    pub in_rate: f64,
    pub dur_secs: f32,
    pub style: Style,
    pub out_bpm_eff: f32,
    /// Per-layer fade windows built from the songs' layers (Phase 3).
    pub script: Option<Script>,
}

#[derive(Clone, Copy)]
pub struct Trans {
    pub out: usize,
    pub inn: usize,
    pub total: u32,
    pub elapsed: u32,
    pub style: Style,
    pub script: Option<Script>,
}

#[derive(Clone, Copy)]
pub struct TP {
    pub gain: f32,
    pub low: f32,
    pub mid: f32,
    pub high: f32,
    pub filt: f32,
    pub echo: f32,
    pub stem: [f32; NL],
}

impl TP {
    pub fn neutral() -> TP {
        TP { gain: 1.0, low: 1.0, mid: 1.0, high: 1.0, filt: 1.0, echo: 0.0, stem: [1.0; NL] }
    }
}

fn sm(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Returns (outgoing, incoming) parameters at transition progress x in 0..1.
pub fn trans_params(style: Style, x: f32) -> (TP, TP) {
    let mut o = TP::neutral();
    let mut i = TP::neutral();
    match style {
        Style::Blend => {
            o.gain = (1.0 - 0.18 * sm(0.4, 0.55, x)) * (sm(0.55, 1.0, x) * PI / 2.0).cos();
            o.low = 1.0 - sm(0.42, 0.56, x);
            o.mid = 1.0 - 0.6 * sm(0.45, 1.0, x);
            o.high = 1.0 - 0.8 * sm(0.5, 1.0, x);
            i.gain = (0.92 + 0.08 * sm(0.5, 0.7, x)) * (sm(0.0, 0.5, x) * PI / 2.0).sin();
            i.low = sm(0.42, 0.56, x);
            i.mid = 0.3 + 0.7 * sm(0.1, 0.6, x);
            i.high = 0.4 + 0.6 * sm(0.0, 0.5, x);
            o.stem = [1.0 - sm(0.12, 0.38, x), 1.0 - 0.5 * sm(0.45, 1.0, x), 1.0 - sm(0.42, 0.56, x), 1.0 - sm(0.5, 0.95, x), 1.0 - 0.5 * sm(0.45, 1.0, x)];
            i.stem = [sm(0.62, 0.9, x), sm(0.1, 0.55, x), sm(0.42, 0.56, x), sm(0.0, 0.3, x), sm(0.1, 0.55, x)];
        }
        Style::FilterEcho => {
            o.gain = (sm(0.8, 1.0, x) * PI / 2.0).cos();
            o.filt = 1.0 - 0.9 * sm(0.1, 0.85, x);
            o.echo = 0.7 * sm(0.3, 0.5, x);
            i.gain = (sm(0.4, 0.85, x) * PI / 2.0).sin();
            i.low = sm(0.5, 0.7, x);
            i.filt = 0.25 + 0.75 * sm(0.4, 1.0, x);
            o.stem = [1.0 - sm(0.1, 0.35, x), 1.0, 1.0 - sm(0.4, 0.7, x), 1.0, 1.0];
            i.stem = [sm(0.65, 0.95, x), 1.0, sm(0.5, 0.7, x), sm(0.0, 0.3, x), 1.0];
        }
        Style::EchoOut => {
            o.gain = (sm(0.3, 0.42, x) * PI / 2.0).cos();
            o.echo = 0.9 * sm(0.05, 0.3, x);
            o.filt = 1.0 - 0.5 * sm(0.1, 0.4, x);
            i.gain = if x < 0.45 { 0.0 } else { sm(0.45, 0.5, x) };
            o.stem = [1.0 - sm(0.05, 0.3, x), 1.0, 1.0, 1.0, 1.0];
        }
        Style::Plain => {
            o.gain = (x * PI / 2.0).cos();
            i.gain = (x * PI / 2.0).sin();
        }
        Style::Fade => {
            // Songs without a shared beat: never let two basslines or two singers overlap,
            // and add a little gain in the middle to make up for what the EQ takes away.
            let t = sm(0.0, 1.0, x);
            let comp = 1.0 + 0.15 * (x * PI).sin();
            o.gain = (t * PI / 2.0).cos() * comp;
            i.gain = (t * PI / 2.0).sin() * comp;
            let s = sm(0.4, 0.6, x);
            o.low = (s * PI / 2.0).cos();
            i.low = (s * PI / 2.0).sin();
            o.high = 1.0 - 0.35 * sm(0.4, 1.0, x);
            i.high = 0.75 + 0.25 * sm(0.0, 0.5, x);
            o.filt = 1.0 - 0.3 * sm(0.4, 1.0, x);
            // with layers: vocals leave early and enter late, drums never clash
            o.stem = [1.0 - sm(0.12, 0.42, x), 1.0, 1.0 - sm(0.38, 0.58, x), 1.0 - sm(0.25, 0.55, x), 1.0];
            i.stem = [sm(0.6, 0.9, x), 1.0, sm(0.38, 0.58, x), sm(0.45, 0.75, x), 1.0];
        }
    }
    (o, i)
}

const WN: usize = 1024;
const WH: usize = 512;
const WS: isize = 48;

fn at(p: &[f32], i: isize) -> f32 {
    if i < 0 || i as usize >= p.len() {
        0.0
    } else {
        p[i as usize]
    }
}

fn fr4(p: &[f32], st: Option<&Stems>, i: isize) -> [f32; NL] {
    match st {
        Some(s) => std::array::from_fn(|k| at(&s.data[k], i)),
        None => {
            let mut a = [0.0; NL];
            a[0] = at(p, i);
            a
        }
    }
}

fn mul4(a: [f32; NL], w: f32) -> [f32; NL] {
    a.map(|x| x * w)
}

fn add4(a: [f32; NL], b: [f32; NL]) -> [f32; NL] {
    std::array::from_fn(|k| a[k] + b[k])
}

/// Waveform-similarity overlap-add time stretcher (tempo change, pitch kept).
pub struct Stretch {
    apos: f64,
    prev_nat: isize,
    first: bool,
    tail: Vec<[f32; NL]>,
    win: Vec<f32>,
    q: Vec<[f32; NL]>,
    qi: usize,
}

impl Stretch {
    fn new() -> Stretch {
        let win = (0..WN).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / WN as f32).cos()).collect();
        Stretch { apos: 0.0, prev_nat: 0, first: true, tail: vec![[0.0; NL]; WH], win, q: Vec::new(), qi: 0 }
    }
    fn reset(&mut self, pos: f64) {
        self.apos = pos;
        self.first = true;
        self.q.clear();
        self.qi = 0;
    }
    fn next(&mut self, p: &[f32], st: Option<&Stems>, rate: f64) -> [f32; NL] {
        if self.qi >= self.q.len() {
            self.gen(p, st, rate);
        }
        let v = self.q[self.qi];
        self.qi += 1;
        v
    }
    fn gen(&mut self, p: &[f32], st: Option<&Stems>, rate: f64) {
        self.q.clear();
        self.qi = 0;
        let target = self.apos.round() as isize;
        if self.first {
            let c = target.max(0);
            for i in 0..WH {
                self.q.push(fr4(p, st, c + i as isize));
            }
            for i in 0..WH {
                self.tail[i] = mul4(fr4(p, st, c + (WH + i) as isize), self.win[WH + i]);
            }
            self.prev_nat = c + WH as isize;
            self.first = false;
        } else {
            let mut best = target;
            let mut bs = f32::MIN;
            let mut d = -WS;
            while d <= WS {
                let c = target + d;
                if c >= 0 {
                    let mut s = 0f32;
                    let mut k = 0usize;
                    while k < WH {
                        s += at(p, c + k as isize) * at(p, self.prev_nat + k as isize);
                        k += 4;
                    }
                    if s > bs {
                        bs = s;
                        best = c;
                    }
                }
                d += 1;
            }
            for i in 0..WH {
                self.q.push(add4(self.tail[i], mul4(fr4(p, st, best + i as isize), self.win[i])));
                self.tail[i] = mul4(fr4(p, st, best + (WH + i) as isize), self.win[WH + i]);
            }
            self.prev_nat = best + WH as isize;
        }
        self.apos += WH as f64 * rate;
    }
}

pub struct Deck {
    pub pcm: Option<Arc<Vec<f32>>>,
    pub track: Option<usize>,
    pub pos: f64,
    pub rate: f64,
    pub target_rate: f64,
    pub playing: bool,
    pub norm: f32,
    pub bpm: f32,
    pub origin: f64,
    pub eq: [f32; 3],
    pub filt: f32,
    pub loop_on: bool,
    pub loop_a: f64,
    pub loop_b: f64,
    pub cues: [Option<f64>; 4],
    pub level: f32,
    pub ease: bool,
    /// Beat sync: keep this deck locked (tempo and beat phase) to the other playing deck.
    pub sync: bool,
    /// Automatic lock for the incoming deck during a beat-matched mix.
    pub lock_auto: bool,
    /// Rate that matches the other deck's tempo while locked (before phase correction).
    pub lock_base: f64,
    /// Wanted beat offset to the other deck, in this deck's samples (0 = on the grid).
    pub phase_goal: f64,
    lp1: f32,
    st: Stretch,
    s0: [f32; NL],
    s1: [f32; NL],
    pub stems: Option<Arc<Stems>>,
    pub sg: [f32; NL],
    sgc: [f32; NL],
    stems_on: bool,
    declick: u32,
    frac: f64,
    expect: f64,
    stretching: bool,
    need_reset: bool,
    lp2: f32,
    f1: f32,
    f2: f32,
    a1: f32,
    a2: f32,
}

impl Deck {
    /// Reads one output sample. When the deck runs faster or slower than
    /// normal the audio is time-stretched (WSOLA) so the pitch stays put.
    /// With separated layers the four layers are mixed with their own gains.
    fn read_sample(&mut self, p: &[f32], stems: Option<&Stems>, out_sr: f32) -> Option<f32> {
        let has = stems.is_some();
        if has != self.stems_on {
            self.stems_on = has;
            self.need_reset = true;
            self.declick = 192;
        }
        if (self.pos - self.expect).abs() > 3.0 {
            self.need_reset = true;
        }
        let i = self.pos as usize;
        if i + 1 >= p.len() {
            return None;
        }
        let g = self.sgc;
        let want = (self.rate - 1.0).abs() > 0.0003;
        if !want {
            self.stretching = false;
            self.need_reset = true;
            let fr = (self.pos - i as f64) as f32;
            return Some(match stems {
                Some(s) => {
                    let mut v = 0.0;
                    for k in 0..NL {
                        let d = &s.data[k];
                        v += (d[i] * (1.0 - fr) + d[i + 1] * fr) * g[k];
                    }
                    v
                }
                None => p[i] * (1.0 - fr) + p[i + 1] * fr,
            });
        }
        if !self.stretching || self.need_reset {
            self.stretching = true;
            self.need_reset = false;
            self.st.reset(self.pos);
            self.s0 = self.st.next(p, stems, self.rate);
            self.s1 = self.st.next(p, stems, self.rate);
            self.frac = 0.0;
        }
        let f = self.frac as f32;
        let mut v = 0.0;
        if has {
            for k in 0..NL {
                v += (self.s0[k] + (self.s1[k] - self.s0[k]) * f) * g[k];
            }
        } else {
            v = self.s0[0] + (self.s1[0] - self.s0[0]) * f;
        }
        self.frac += SR as f64 / out_sr as f64;
        while self.frac >= 1.0 {
            self.frac -= 1.0;
            self.s0 = self.s1;
            self.s1 = self.st.next(p, stems, self.rate);
        }
        Some(v)
    }

    fn new() -> Deck {
        Deck {
            pcm: None,
            track: None,
            pos: 0.0,
            rate: 1.0,
            target_rate: 1.0,
            playing: false,
            norm: 1.0,
            bpm: 120.0,
            origin: 0.0,
            eq: [1.0; 3],
            filt: 1.0,
            loop_on: false,
            loop_a: 0.0,
            loop_b: 0.0,
            cues: [None; 4],
            level: 0.0,
            ease: false,
            sync: false,
            lock_auto: false,
            lock_base: 1.0,
            phase_goal: 0.0,
            lp1: 0.0,
            st: Stretch::new(),
            s0: [0.0; NL],
            s1: [0.0; NL],
            stems: None,
            sg: [1.0; NL],
            sgc: [1.0; NL],
            stems_on: false,
            declick: 0,
            frac: 0.0,
            expect: 0.0,
            stretching: false,
            need_reset: true,
            lp2: 0.0,
            f1: 0.0,
            f2: 0.0,
            a1: 0.03,
            a2: 0.3,
        }
    }
    pub fn len(&self) -> f64 {
        self.pcm.as_ref().map(|p| p.len() as f64).unwrap_or(0.0)
    }
    pub fn eff_bpm(&self) -> f32 {
        self.bpm * self.rate as f32
    }
    pub fn spb(&self) -> f64 {
        SR as f64 * 60.0 / self.bpm as f64
    }
    fn set_sr(&mut self, sr: f32) {
        self.a1 = 1.0 - (-2.0 * PI * 250.0 / sr).exp();
        self.a2 = 1.0 - (-2.0 * PI * 2500.0 / sr).exp();
    }
    /// Returns (dry output after gain, echo send).
    fn process(&mut self, out_sr: f32, tp: &TP) -> (f32, f32) {
        let pcm = match self.pcm.take() {
            Some(p) => p,
            None => return (0.0, 0.0),
        };
        let stems = self.stems.take();
        for k in 0..NL {
            let target = self.sg[k] * tp.stem[k];
            self.sgc[k] += (target - self.sgc[k]) * 0.003;
        }
        let s = self.read_sample(&pcm, stems.as_deref(), out_sr);
        self.pcm = Some(pcm);
        self.stems = stems;
        let mut s = match s {
            Some(v) => v,
            None => {
                self.playing = false;
                return (0.0, 0.0);
            }
        };
        if self.declick > 0 {
            s *= 1.0 - self.declick as f32 / 192.0;
            self.declick -= 1;
        }
        let inc = SR as f64 / out_sr as f64;
        self.pos += self.rate * inc;
        if self.loop_on && self.pos >= self.loop_b {
            self.pos = self.loop_a + (self.pos - self.loop_b);
            self.need_reset = true;
        }
        self.expect = self.pos;
        if self.ease {
            let st = 0.004 / out_sr as f64;
            if (self.target_rate - self.rate).abs() <= st {
                self.rate = self.target_rate;
                self.ease = false;
            } else if self.target_rate > self.rate {
                self.rate += st;
            } else {
                self.rate -= st;
            }
        }
        let x = s * self.norm;
        self.lp1 += self.a1 * (x - self.lp1);
        self.lp2 += self.a2 * (x - self.lp2);
        let low = self.lp1;
        let mid = self.lp2 - self.lp1;
        let high = x - self.lp2;
        let (tl, tm, th) = if self.stems_on { (1.0, 1.0, 1.0) } else { (tp.low, tp.mid, tp.high) };
        let mut y = low * self.eq[0] * tl + mid * self.eq[1] * tm + high * self.eq[2] * th;
        let f = tp.filt.min(self.filt);
        if f < 0.98 {
            let fc = 60.0 * (16000.0f32 / 60.0).powf(f.max(0.0));
            let a = 1.0 - (-2.0 * PI * fc / out_sr).exp();
            self.f1 += a * (y - self.f1);
            self.f2 += a * (self.f1 - self.f2);
            y = self.f2;
        } else {
            self.f1 = y;
            self.f2 = y;
        }
        let out = y * tp.gain;
        self.level = (self.level * 0.9995).max(out.abs());
        (out, y * tp.echo)
    }
}

pub struct Mixer {
    pub sr: f32,
    pub decks: [Deck; 2],
    pub active: usize,
    pub xfader: f32,
    pub manual: bool,
    pub master: f32,
    pub pending: Option<Pending>,
    pub trans: Option<Trans>,
    pub completed: u32,
    pub scope: Vec<f32>,
    pub scope_head: usize,
    pub peak_seen: f32,
    pub max_jump: f32,
    pub frames_rendered: u64,
    last: f32,
    dl: Vec<f32>,
    dr: Vec<f32>,
    wpos: usize,
    delay: usize,
    pub echo_fb: f32,
    pub paused: bool,
    lpo: [f32; 2],
    lpa: f32,
    /// DJ (headphone) output: on/off, false = cue the up-next song, true = monitor the main mix.
    pub dj_on: bool,
    pub dj_mix: bool,
    pub dj_vol: f32,
    cue_pos: f64,
    cue_track: Option<usize>,
    cue_seen: u32,
    cue_stale: Option<usize>,
    mon: std::collections::VecDeque<(f32, f32)>,
    mon_a: (f32, f32),
    mon_b: (f32, f32),
    mon_ph: f32,
    /// Snap to beat: jumps (cue, waveform clicks, play) keep the beat phase of the other deck.
    pub quantize: bool,
    sync_ctr: u32,
}

impl Mixer {
    pub fn new() -> Mixer {
        let mut m = Mixer {
            sr: 44100.0,
            decks: [Deck::new(), Deck::new()],
            active: 0,
            xfader: 0.0,
            manual: false,
            master: 0.8,
            pending: None,
            trans: None,
            completed: 0,
            scope: vec![0.0; 2048],
            scope_head: 0,
            peak_seen: 0.0,
            max_jump: 0.0,
            frames_rendered: 0,
            last: 0.0,
            dl: Vec::new(),
            dr: Vec::new(),
            wpos: 0,
            delay: 10000,
            echo_fb: 0.5,
            paused: false,
            lpo: [0.0; 2],
            lpa: 1.0,
            dj_on: false,
            dj_mix: false,
            dj_vol: 0.8,
            cue_pos: 0.0,
            cue_track: None,
            cue_seen: 0,
            cue_stale: None,
            mon: std::collections::VecDeque::new(),
            mon_a: (0.0, 0.0),
            mon_b: (0.0, 0.0),
            mon_ph: 0.0,
            quantize: true,
            sync_ctr: 0,
        };
        m.set_sr(44100.0);
        m
    }

    pub fn reset(&mut self) {
        let sr = self.sr;
        let master = self.master;
        let (dj_on, dj_mix, dj_vol) = (self.dj_on, self.dj_mix, self.dj_vol);
        *self = Mixer::new();
        self.set_sr(sr);
        self.master = master;
        self.dj_on = dj_on;
        self.dj_mix = dj_mix;
        self.dj_vol = dj_vol;
    }

    pub fn set_sr(&mut self, sr: f32) {
        self.sr = sr;
        self.lpa = if sr > 30000.0 { 1.0 - (-2.0 * PI * 10500.0 / sr).exp() } else { 1.0 };
        for d in self.decks.iter_mut() {
            d.set_sr(sr);
        }
        let n = (sr * 2.5) as usize;
        self.dl = vec![0.0; n];
        self.dr = vec![0.0; n];
        self.wpos = 0;
        self.delay = (sr * 0.4) as usize;
    }

    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let frames = out.len() / channels.max(1);
        if self.paused {
            for v in out.iter_mut() {
                *v = 0.0;
            }
            return;
        }
        for f in 0..frames {
            self.tick();
            let (l, r) = self.frame();
            if self.dj_on && self.dj_mix {
                if self.mon.len() > (self.sr * 0.4) as usize {
                    self.mon.pop_front();
                }
                self.mon.push_back((l, r));
            }
            for c in 0..channels {
                out[f * channels + c] = if c == 0 {
                    l
                } else if c == 1 {
                    r
                } else {
                    0.0
                };
            }
        }
    }

    /// Output for the DJ's own device. Cue mode pre-listens the up-next song from a private
    /// cursor (the audience never hears it); mix mode monitors the main mix.
    pub fn render_cue(&mut self, out: &mut [f32], channels: usize, out_sr: f32) {
        let ch = channels.max(1);
        let frames = out.len() / ch;
        let silent = |out: &mut [f32]| out.iter_mut().for_each(|v| *v = 0.0);
        if !self.dj_on {
            self.mon.clear();
            return silent(out);
        }
        if self.dj_mix {
            let step = self.sr / out_sr.max(1.0);
            for f in 0..frames {
                self.mon_ph += step;
                while self.mon_ph >= 1.0 {
                    self.mon_ph -= 1.0;
                    self.mon_a = self.mon_b;
                    self.mon_b = self.mon.pop_front().unwrap_or((0.0, 0.0));
                }
                let p = self.mon_ph;
                let g = (self.dj_vol / self.master.max(0.1)).min(8.0); // independent of the audience volume
                let l = self.mon_a.0 + (self.mon_b.0 - self.mon_a.0) * p;
                let r = self.mon_a.1 + (self.mon_b.1 - self.mon_a.1) * p;
                for c in 0..ch {
                    out[f * ch + c] = if c == 0 { l * g } else if c == 1 { r * g } else { 0.0 };
                }
            }
            return;
        }
        self.mon.clear();
        let cd = 1 - self.active;
        if self.completed != self.cue_seen {
            // the decks just swapped: ignore the finished song until a new one is loaded
            self.cue_seen = self.completed;
            self.cue_stale = self.decks[cd].track;
            self.cue_track = None;
        }
        let d = &self.decks[cd];
        let p = match (&d.pcm, d.track) {
            (Some(p), t) if t.is_some() && t != self.cue_stale => p.clone(),
            _ => return silent(out),
        };
        if self.cue_track != d.track {
            self.cue_track = d.track;
            self.cue_pos = d.pos;
        }
        let (norm, origin) = (d.norm, d.origin);
        let inc = SR as f64 / out_sr.max(1.0) as f64;
        for f in 0..frames {
            let i = self.cue_pos as usize;
            let v = if i + 1 < p.len() {
                let fr = (self.cue_pos - i as f64) as f32;
                (p[i] * (1.0 - fr) + p[i + 1] * fr) * norm * self.dj_vol
            } else {
                self.cue_pos = origin; // loop the preview
                0.0
            };
            self.cue_pos += inc;
            let v = (v * 0.9).tanh();
            for c in 0..ch {
                out[f * ch + c] = if c < 2 { v } else { 0.0 };
            }
        }
    }

    /// Move the DJ preview (headphones) to `pos` in song `track`. The audience mix is untouched.
    pub fn cue_seek(&mut self, track: usize, pos: f64) {
        let cd = 1 - self.active;
        if self.decks[cd].track == Some(track) {
            self.cue_pos = pos.max(0.0);
            self.cue_track = Some(track);
        }
    }

    /// Where the DJ preview currently is, if it is playing a song.
    pub fn cue_position(&self) -> Option<(usize, f64)> {
        if self.dj_on && !self.dj_mix {
            self.cue_track.map(|t| (t, self.cue_pos))
        } else {
            None
        }
    }

    /// Restart the pre-listen from the song's planned entry point.
    pub fn cue_restart(&mut self) {
        self.cue_track = None;
    }

    /// Offline render of `secs` seconds (used when no audio device exists).
    pub fn render_virtual(&mut self, secs: f32) {
        let mut buf = vec![0f32; 2048 * 2];
        let mut left = (secs * self.sr) as usize;
        while left > 0 {
            let n = left.min(2048);
            self.render(&mut buf[..n * 2], 2);
            left -= n;
        }
    }

    /// Beat relation of deck `s` to the other deck while that one plays:
    /// (rate that matches its tempo, samples `s` must move forward to sit on its beat
    /// grid, length of one alignment unit in `s` samples). Half and double time are
    /// matched to whichever needs the smaller tempo change. `bars` aligns downbeats
    /// (one 4-beat bar) instead of single beats.
    pub fn phase_target(&self, s: usize, bars: bool) -> Option<(f64, f64, f64)> {
        let m = 1 - s;
        let (ds, dm) = (&self.decks[s], &self.decks[m]);
        if !dm.playing || dm.pcm.is_none() || ds.pcm.is_none() {
            return None;
        }
        let em = dm.eff_bpm() as f64;
        let bs = ds.bpm as f64;
        if em < 30.0 || bs < 30.0 {
            return None;
        }
        let mut mult = 1.0f64;
        for c in [0.5f64, 2.0] {
            if (em * c / bs).ln().abs() < (em * mult / bs).ln().abs() {
                mult = c;
            }
        }
        let rate = em * mult / bs;
        let bm = (dm.pos - dm.origin) / dm.spb();
        let bsl = (ds.pos - ds.origin) / ds.spb();
        let k = if bars { 4.0 } else { 1.0 };
        // Phase over the longer of the two beats (in master beats and slave beats).
        let (pm, ps, unit_beats) = if mult >= 1.0 { ((bm / k).rem_euclid(1.0), (bsl / (mult * k)).rem_euclid(1.0), mult * k) } else { ((bm * mult / k).rem_euclid(1.0), (bsl / k).rem_euclid(1.0), k) };
        let mut e = pm - ps;
        if e > 0.5 {
            e -= 1.0;
        } else if e < -0.5 {
            e += 1.0;
        }
        let unit = unit_beats * ds.spb();
        Some((rate, e * unit, unit))
    }

    /// Jumps deck `d` onto the other deck's beat (or bar) phase. Returns false if
    /// there is nothing to line up with.
    pub fn snap_phase(&mut self, d: usize, bars: bool) -> bool {
        match self.phase_target(d, bars) {
            Some((_, mv, unit)) => {
                let len = self.decks[d].len();
                let dk = &mut self.decks[d];
                // Stay inside the song by a whole unit, so the phase is still right.
                let mut p = dk.pos + mv;
                if p < 0.0 {
                    p += unit;
                }
                if p > len - 1.0 {
                    p -= unit;
                }
                dk.pos = p.clamp(0.0, (len - 1.0).max(0.0));
                dk.need_reset = true;
                dk.declick = 192;
                dk.phase_goal = 0.0;
                true
            }
            None => false,
        }
    }

    /// Manual Sync: match tempo, jump onto the other deck's bar, and stay locked.
    pub fn sync_deck(&mut self, d: usize) -> bool {
        let Some((rate, _, _)) = self.phase_target(d, true) else { return false };
        self.snap_phase(d, true);
        let dk = &mut self.decks[d];
        dk.rate = rate;
        dk.target_rate = rate;
        dk.lock_base = rate;
        dk.ease = false;
        dk.sync = true;
        true
    }

    /// Shift the beat lock of deck `d` by `samples` (positive = later in its song).
    /// The deck glides there by bending its tempo slightly; no audible jump.
    pub fn nudge_phase(&mut self, d: usize, samples: f64) {
        self.decks[d].phase_goal -= samples;
    }

    /// Phase-locked loop, run every 128 frames: holds each locked deck on the other
    /// deck's beat grid (plus its wanted offset) with a small tempo bend (max 4%, pitch stays).
    fn sync_tick(&mut self) {
        for s in 0..2 {
            let in_mix = self.trans.map(|t| t.inn == s).unwrap_or(false);
            if self.decks[s].lock_auto && !in_mix {
                self.decks[s].lock_auto = false;
            }
            let manual = self.decks[s].sync;
            let auto = self.decks[s].lock_auto;
            if !(manual || auto) || !self.decks[s].playing {
                continue;
            }
            let Some((rate, err, unit)) = self.phase_target(s, false) else { continue };
            // Manual lock follows the other deck's tempo; in a mix the DJ's planned rate holds.
            let base = if manual && !auto { rate } else { self.decks[s].lock_base };
            let mut e = err - self.decks[s].phase_goal;
            e -= (e / unit).round() * unit;
            // Seconds of offset in the song's own time; settles in about a second.
            let secs = e / SR as f64;
            let bend = (secs * 1.5).clamp(-0.04, 0.04);
            let dk = &mut self.decks[s];
            dk.lock_base = base;
            dk.rate = base * (1.0 + bend);
            dk.target_rate = dk.rate;
            dk.ease = false;
        }
    }

    /// Beat offset of the incoming / locked deck to the other deck, in milliseconds
    /// (for the phase meter). None when the decks cannot be compared.
    pub fn phase_ms(&self, d: usize) -> Option<f32> {
        let (_, mv, _) = self.phase_target(d, false)?;
        Some((-mv / SR as f64 / self.decks[d].rate.max(0.1) * 1000.0) as f32)
    }

    fn tick(&mut self) {
        self.sync_ctr += 1;
        if self.sync_ctr >= 128 {
            self.sync_ctr = 0;
            self.sync_tick();
        }
        if let Some(p) = self.pending {
            if self.trans.is_none() {
                let a = self.active;
                if !self.manual && (self.decks[a].pos >= p.trigger || !self.decks[a].playing) {
                    self.start_transition(p);
                }
            }
        }
    }

    fn start_transition(&mut self, p: Pending) {
        let out = self.active;
        let inn = p.deck_in;
        {
            let d = &mut self.decks[inn];
            d.pos = p.start_pos;
            d.need_reset = true;
            d.rate = p.in_rate;
            d.target_rate = p.in_rate;
            d.playing = true;
            d.ease = false;
            d.eq = [1.0; 3];
            d.filt = 1.0;
        }
        let beat = 60.0 / p.out_bpm_eff.max(40.0);
        self.delay = (((beat * 0.75) * self.sr) as usize).clamp(1000, self.dl.len() - 1);
        self.trans = Some(Trans { out, inn, total: (p.dur_secs * self.sr).max(1.0) as u32, elapsed: 0, style: p.style, script: p.script });
        // A mix hands over control: no manual lock on either deck from here on.
        self.decks[out].sync = false;
        self.decks[inn].sync = false;
        // Beat-matched mix: lock the incoming deck to the outgoing beat so the kicks
        // never drift apart, holding the alignment the DJ chose for the entry.
        self.decks[inn].lock_auto = false;
        if let Some((rate, err, _)) = self.phase_target(inn, false) {
            if (rate / p.in_rate - 1.0).abs() < 0.003 {
                let d = &mut self.decks[inn];
                d.lock_auto = true;
                d.lock_base = p.in_rate;
                d.phase_goal = err;
            }
        }
        self.pending = None;
    }

    fn frame(&mut self) -> (f32, f32) {
        let sr = self.sr;
        let mut tp = [TP::neutral(), TP::neutral()];
        if let Some(t) = self.trans {
            let x = t.elapsed as f32 / t.total as f32;
            let (mut o, mut i) = trans_params(t.style, x);
            if let Some(sc) = t.script {
                if self.decks[t.out].stems.is_some() && self.decks[t.inn].stems.is_some() {
                    apply_script(&sc, x, &mut o, &mut i);
                }
            }
            tp[t.out] = o;
            tp[t.inn] = i;
            // keep the crossfader display following the automatic mix
            let target = if t.inn == 1 { x } else { 1.0 - x };
            self.xfader = target;
        } else if self.manual {
            tp[0].gain = (self.xfader * PI / 2.0).cos();
            tp[1].gain = (self.xfader * PI / 2.0).sin();
        }
        let mut sum = 0f32;
        let mut send = 0f32;
        for d in 0..2 {
            if self.decks[d].playing {
                let (o, e) = self.decks[d].process(sr, &tp[d]);
                sum += o;
                send += e;
            } else {
                self.decks[d].level *= 0.999;
            }
        }
        // echo bus (ping-pong)
        let n = self.dl.len();
        let ri = (self.wpos + n - self.delay) % n;
        let el = self.dl[ri];
        let er = self.dr[ri];
        self.dl[self.wpos] = send + self.echo_fb * er;
        self.dr[self.wpos] = self.echo_fb * el;
        self.wpos = (self.wpos + 1) % n;
        let mono = (sum + 0.0) * self.master;
        let la = mono + el * 0.8 * self.master;
        let ra = mono + er * 0.8 * self.master;
        self.lpo[0] += self.lpa * (la - self.lpo[0]);
        self.lpo[1] += self.lpa * (ra - self.lpo[1]);
        let l = (self.lpo[0] * 0.9).tanh();
        let r = (self.lpo[1] * 0.9).tanh();
        // metering + continuity check
        self.peak_seen = self.peak_seen.max(l.abs());
        let j = (l - self.last).abs();
        if j > self.max_jump && self.frames_rendered > 10 {
            self.max_jump = j;
        }
        self.last = l;
        self.frames_rendered += 1;
        self.scope[self.scope_head] = (l + r) * 0.5;
        self.scope_head = (self.scope_head + 1) % self.scope.len();
        if let Some(t) = self.trans.as_mut() {
            t.elapsed += 1;
            if t.elapsed >= t.total {
                let t = *t;
                self.decks[t.out].playing = false;
                self.decks[t.out].level = 0.0;
                self.active = t.inn;
                let d = &mut self.decks[t.inn];
                d.ease = true;
                d.target_rate = 1.0;
                self.completed += 1;
                self.trans = None;
            }
        }
        (l, r)
    }

    pub fn trans_progress(&self) -> Option<f32> {
        self.trans.map(|t| t.elapsed as f32 / t.total as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize) -> Arc<Vec<f32>> {
        Arc::new((0..n).map(|i| (i as f32 * 0.05).sin() * 0.5).collect())
    }

    fn mixer_with_decks() -> Mixer {
        let mut m = Mixer::new();
        m.set_sr(48000.0);
        for d in 0..2 {
            m.decks[d].pcm = Some(tone(SR as usize * 20));
            m.decks[d].track = Some(d);
            m.decks[d].norm = 1.0;
        }
        m.decks[0].playing = true;
        m.decks[1].pos = 1000.0;
        m
    }

    #[test]
    fn sync_locks_tempo_and_beat_phase() {
        let mut m = mixer_with_decks();
        m.decks[0].bpm = 124.0;
        m.decks[0].rate = 1.0;
        m.decks[1].bpm = 120.0;
        m.decks[1].playing = true;
        m.decks[1].pos = 5000.0;
        m.decks[1].rate = 1.0;
        m.decks[0].pos = 30000.0;
        assert!(m.sync_deck(1));
        // tempo matched, and on the bar right after the jump
        assert!((m.decks[1].eff_bpm() - 124.0).abs() < 0.01);
        assert!(m.phase_ms(1).unwrap().abs() < 1.0);
        // push it a third of a beat off: the lock pulls it back without a jump
        m.decks[1].pos += 0.3 * m.decks[1].spb();
        m.decks[1].need_reset = true;
        let before = m.phase_ms(1).unwrap().abs();
        assert!(before > 100.0);
        m.render_virtual(6.0);
        let after = m.phase_ms(1).unwrap().abs();
        assert!(after < 5.0, "phase {} ms -> {} ms", before, after);
        assert!((m.decks[1].eff_bpm() - 124.0).abs() < 0.5);
    }

    #[test]
    fn sync_handles_double_time() {
        let mut m = mixer_with_decks();
        m.decks[0].bpm = 70.0;
        m.decks[1].bpm = 138.0;
        m.decks[1].playing = true;
        assert!(m.sync_deck(1));
        assert!((m.decks[1].eff_bpm() - 140.0).abs() < 0.01);
        m.render_virtual(3.0);
        assert!(m.phase_ms(1).unwrap().abs() < 5.0);
    }

    #[test]
    fn dj_off_is_silent() {
        let mut m = mixer_with_decks();
        let mut out = vec![1.0f32; 960];
        m.render_cue(&mut out, 2, 48000.0);
        assert!(out.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn cue_plays_next_deck_without_touching_the_audience_mix() {
        let mut m = mixer_with_decks();
        m.dj_on = true;
        m.dj_mix = false;
        let pos_before = m.decks[1].pos;
        let mut out = vec![0.0f32; 4800];
        m.render_cue(&mut out, 2, 48000.0);
        assert!(out.iter().any(|v| v.abs() > 0.05), "cue should be audible");
        assert_eq!(m.decks[1].pos, pos_before, "cue must not move the real deck");
        assert_eq!(m.decks[1].playing, false);
        // left and right carry the same signal
        assert!(out.chunks(2).all(|f| f[0] == f[1]));
    }

    #[test]
    fn cue_is_independent_of_audience_render() {
        let mut m = mixer_with_decks();
        m.dj_on = true;
        let mut a = vec![0.0f32; 2400];
        m.render(&mut a, 2);
        let mut c = vec![0.0f32; 2400];
        m.render_cue(&mut c, 2, 48000.0);
        assert!(c.iter().any(|v| v.abs() > 0.05));
        assert!(m.mon.is_empty(), "preview mode must not buffer the main mix");
    }

    #[test]
    fn mix_mode_mirrors_the_main_output() {
        let mut m = mixer_with_decks();
        m.dj_on = true;
        m.dj_mix = true;
        m.dj_vol = 1.0;
        m.master = 1.0;
        let mut main = vec![0.0f32; 4800];
        m.render(&mut main, 2);
        let mut dj = vec![0.0f32; 4800];
        m.render_cue(&mut dj, 2, 48000.0);
        let peak_main = main.iter().fold(0f32, |a, b| a.max(b.abs()));
        let peak_dj = dj.iter().fold(0f32, |a, b| a.max(b.abs()));
        assert!(peak_main > 0.05 && peak_dj > 0.05, "{} {}", peak_main, peak_dj);
        assert!((peak_main - peak_dj).abs() < 0.05);
    }

    #[test]
    fn finished_song_is_not_replayed_in_preview() {
        let mut m = mixer_with_decks();
        m.dj_on = true;
        let mut out = vec![0.0f32; 960];
        m.render_cue(&mut out, 2, 48000.0);
        // decks swap: deck 1 becomes active, deck 0 (old song) becomes the cue deck
        m.active = 1;
        m.completed += 1;
        let mut out = vec![1.0f32; 960];
        m.render_cue(&mut out, 2, 48000.0);
        assert!(out.iter().all(|v| *v == 0.0), "old song must stay silent until a new one loads");
        m.decks[0].track = Some(5);
        m.decks[0].pos = 500.0;
        m.render_cue(&mut out, 2, 48000.0);
        assert!(out.iter().any(|v| v.abs() > 0.05));
    }

    #[test]
    fn different_device_rate_keeps_pitch() {
        // 44.1k cue device: advance per frame must be SR/out_sr
        let mut m = mixer_with_decks();
        m.dj_on = true;
        let mut out = vec![0.0f32; 2 * 44100];
        m.render_cue(&mut out, 2, 44100.0);
        let moved = m.cue_pos - 1000.0;
        assert!((moved - SR as f64).abs() < 2.0, "moved {}", moved);
    }
}

#[cfg(test)]
mod cue_seek_tests {
    use super::*;

    #[test]
    fn ctrl_scrub_moves_only_the_preview() {
        let mut m = Mixer::new();
        m.set_sr(48000.0);
        for d in 0..2 {
            m.decks[d].pcm = Some(Arc::new((0..SR as usize * 20).map(|i| (i as f32 * 0.05).sin() * 0.5).collect()));
            m.decks[d].track = Some(d);
            m.decks[d].norm = 1.0;
        }
        m.decks[0].playing = true;
        m.decks[1].pos = 1000.0;
        m.dj_on = true;
        let audience_before = (m.decks[0].pos, m.decks[1].pos);
        let mut out = vec![0.0f32; 960];
        m.render_cue(&mut out, 2, 48000.0);
        m.cue_seek(1, 300_000.0);
        let mut out = vec![0.0f32; 960];
        m.render_cue(&mut out, 2, 48000.0);
        let (t, p) = m.cue_position().unwrap();
        assert_eq!(t, 1);
        assert!(p > 300_000.0 && p < 301_000.0, "{}", p);
        assert_eq!((m.decks[0].pos, m.decks[1].pos), audience_before);
        // seeking a song that is not the up-next deck is ignored
        m.cue_seek(0, 5.0);
        assert!(m.cue_position().unwrap().1 > 300_000.0);
    }
}

/// Staged, layer-by-layer transition: each layer crossfades (equal power) over its
/// own window, so beats, bass, instruments and vocals change hands one at a time.
/// The deck gains stay at full because the layers now do the fading.
pub fn apply_script(sc: &Script, x: f32, o: &mut TP, i: &mut TP) {
    for k in 0..NL {
        let (a, b) = sc[0][k];
        let t = if b > a { sm(a, b, x) } else if x >= a { 1.0 } else { 0.0 };
        o.stem[k] = (t * PI / 2.0).cos();
        let (a, b) = sc[1][k];
        let t = if b > a { sm(a, b, x) } else if x >= a { 1.0 } else { 0.0 };
        i.stem[k] = (t * PI / 2.0).sin();
    }
    o.gain = 1.0;
    i.gain = 1.0;
    // A filter sweep or echo on the way out still works on top of the layers.
    i.filt = 1.0;
}
