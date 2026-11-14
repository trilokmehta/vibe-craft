//! Audio playback + mixing engine: two decks with tempo (rate) control,
//! 3-band EQ, filter, echo bus, crossfader, and a sample-accurate transition
//! scheduler. Rendering is device independent so it can also run offline.
use crate::stems::Stems;
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

#[derive(Clone, Copy)]
pub struct Pending {
    pub deck_in: usize,
    pub trigger: f64,
    pub start_pos: f64,
    pub in_rate: f64,
    pub dur_secs: f32,
    pub style: Style,
    pub out_bpm_eff: f32,
}

#[derive(Clone, Copy)]
pub struct Trans {
    pub out: usize,
    pub inn: usize,
    pub total: u32,
    pub elapsed: u32,
    pub style: Style,
}

#[derive(Clone, Copy)]
pub struct TP {
    pub gain: f32,
    pub low: f32,
    pub mid: f32,
    pub high: f32,
    pub filt: f32,
    pub echo: f32,
    pub stem: [f32; 4],
}

impl TP {
    pub fn neutral() -> TP {
        TP { gain: 1.0, low: 1.0, mid: 1.0, high: 1.0, filt: 1.0, echo: 0.0, stem: [1.0; 4] }
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
            o.stem = [1.0 - sm(0.12, 0.38, x), 1.0 - 0.5 * sm(0.45, 1.0, x), 1.0 - sm(0.42, 0.56, x), 1.0 - sm(0.5, 0.95, x)];
            i.stem = [sm(0.62, 0.9, x), sm(0.1, 0.55, x), sm(0.42, 0.56, x), sm(0.0, 0.3, x)];
        }
        Style::FilterEcho => {
            o.gain = (sm(0.8, 1.0, x) * PI / 2.0).cos();
            o.filt = 1.0 - 0.9 * sm(0.1, 0.85, x);
            o.echo = 0.7 * sm(0.3, 0.5, x);
            i.gain = (sm(0.4, 0.85, x) * PI / 2.0).sin();
            i.low = sm(0.5, 0.7, x);
            i.filt = 0.25 + 0.75 * sm(0.4, 1.0, x);
            o.stem = [1.0 - sm(0.1, 0.35, x), 1.0, 1.0 - sm(0.4, 0.7, x), 1.0];
            i.stem = [sm(0.65, 0.95, x), 1.0, sm(0.5, 0.7, x), sm(0.0, 0.3, x)];
        }
        Style::EchoOut => {
            o.gain = (sm(0.3, 0.42, x) * PI / 2.0).cos();
            o.echo = 0.9 * sm(0.05, 0.3, x);
            o.filt = 1.0 - 0.5 * sm(0.1, 0.4, x);
            i.gain = if x < 0.45 { 0.0 } else { sm(0.45, 0.5, x) };
            o.stem = [1.0 - sm(0.05, 0.3, x), 1.0, 1.0, 1.0];
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
            o.stem = [1.0 - sm(0.12, 0.42, x), 1.0, 1.0 - sm(0.38, 0.58, x), 1.0 - sm(0.25, 0.55, x)];
            i.stem = [sm(0.6, 0.9, x), 1.0, sm(0.38, 0.58, x), sm(0.45, 0.75, x)];
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

fn fr4(p: &[f32], st: Option<&Stems>, i: isize) -> [f32; 4] {
    match st {
        Some(s) => [at(&s.data[0], i), at(&s.data[1], i), at(&s.data[2], i), at(&s.data[3], i)],
        None => [at(p, i), 0.0, 0.0, 0.0],
    }
}

fn mul4(a: [f32; 4], w: f32) -> [f32; 4] {
    [a[0] * w, a[1] * w, a[2] * w, a[3] * w]
}

fn add4(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

/// Waveform-similarity overlap-add time stretcher (tempo change, pitch kept).
pub struct Stretch {
    apos: f64,
    prev_nat: isize,
    first: bool,
    tail: Vec<[f32; 4]>,
    win: Vec<f32>,
    q: Vec<[f32; 4]>,
    qi: usize,
}

impl Stretch {
    fn new() -> Stretch {
        let win = (0..WN).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / WN as f32).cos()).collect();
        Stretch { apos: 0.0, prev_nat: 0, first: true, tail: vec![[0.0; 4]; WH], win, q: Vec::new(), qi: 0 }
    }
    fn reset(&mut self, pos: f64) {
        self.apos = pos;
        self.first = true;
        self.q.clear();
        self.qi = 0;
    }
    fn next(&mut self, p: &[f32], st: Option<&Stems>, rate: f64) -> [f32; 4] {
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
    lp1: f32,
    st: Stretch,
    s0: [f32; 4],
    s1: [f32; 4],
    pub stems: Option<Arc<Stems>>,
    pub sg: [f32; 4],
    sgc: [f32; 4],
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
                    for k in 0..4 {
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
            for k in 0..4 {
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
            lp1: 0.0,
            st: Stretch::new(),
            s0: [0.0; 4],
            s1: [0.0; 4],
            stems: None,
            sg: [1.0; 4],
            sgc: [1.0; 4],
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
        for k in 0..4 {
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
        };
        m.set_sr(44100.0);
        m
    }

    pub fn reset(&mut self) {
        let sr = self.sr;
        let master = self.master;
        *self = Mixer::new();
        self.set_sr(sr);
        self.master = master;
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

    fn tick(&mut self) {
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
        self.trans = Some(Trans { out, inn, total: (p.dur_secs * self.sr).max(1.0) as u32, elapsed: 0, style: p.style });
        self.pending = None;
    }

    fn frame(&mut self) -> (f32, f32) {
        let sr = self.sr;
        let mut tp = [TP::neutral(), TP::neutral()];
        if let Some(t) = self.trans {
            let x = t.elapsed as f32 / t.total as f32;
            let (o, i) = trans_params(t.style, x);
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
