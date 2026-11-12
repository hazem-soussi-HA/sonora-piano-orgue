//! The Sonor voice machine: additive organ + piano rendering.

use crate::params::{SonorParams, DRAW_MULT};

const TAU: f32 = std::f32::consts::TAU;

/// xorshift64* — deterministic so a given seed always yields the same timbre.
pub struct Rng {
    s: u64,
}

impl Rng {
    pub fn new(seed: f32) -> Self {
        let mut s = (seed as u64) ^ 0x9E37_79B9_7F4A_7C15;
        if s == 0 {
            s = 0x2545_F491_4F6C_DD1D;
        }
        Rng { s }
    }
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.s;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.s = x;
        (x >> 32) as u32
    }
    pub fn next_f32(&mut self) -> f32 {
        self.next_u32() as f32 / 4_294_967_296.0
    }
    pub fn bipolar(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

/// One additive partial with its own decay behaviour.
struct Partial {
    phase: f32,
    inc: f32,
    env: f32,
    amp: f32,
    decay_mult: f32,
    attack: f32,
    detune_cents: f32,
    base_freq: f32,
}

/// RBJ band-pass biquad — used for the chiff and hammer transients.
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn bandpass(sr: f32, f0: f32, q: f32) -> Self {
        let w = TAU * f0 / sr;
        let alpha = w.sin() / (2.0 * q);
        let cosw = w.cos();
        let a0 = 1.0 + alpha;
        Biquad {
            b0: alpha / a0,
            b1: 0.0,
            b2: -alpha / a0,
            a1: (-2.0 * cosw) / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// One-pole low-pass (cascaded twice for -12 dB/oct).
struct OnePole {
    a: f32,
    z: f32,
}

impl OnePole {
    fn new() -> Self {
        OnePole { a: 1.0, z: 0.0 }
    }
    #[inline]
    fn set_cutoff(&mut self, fc: f32, sr: f32) {
        self.a = 1.0 - (-TAU * fc / sr).exp();
    }
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        self.z += self.a * (x - self.z);
        self.z
    }
}

/// Render one note into `out` (mono). The host applies the release envelope,
/// so this produces the attack + natural decay of the instrument.
pub fn render_note(out: &mut [f32], p: &SonorParams) {
    let sr = p.sample_rate.max(8_000.0);
    let n = out.len();
    if n == 0 {
        return;
    }
    let vel = p.vel.clamp(0.05, 1.0);
    let f0 = 440.0 * ((p.midi - 69.0) / 12.0).exp2();
    let mut rng = Rng::new(p.seed * 977.0 + p.midi * 13.0);

    /* ---------------- organ partials ---------------- */
    let organ_gain = (1.0 - p.blend) * p.o_level;
    let mut organ: Vec<Partial> = Vec::new();
    if organ_gain > 0.002 {
        for i in 0..8 {
            let lvl = p.draw[i];
            if lvl <= 0.0 {
                continue;
            }
            let freq = f0 * DRAW_MULT[i];
            if freq > sr * 0.46 {
                continue;
            }
            let amp = (lvl / 8.0).powf(1.55) * 0.34;
            organ.push(Partial {
                phase: rng.next_f32(),
                inc: freq / sr,
                env: 1.0,
                amp,
                decay_mult: 1.0,
                attack: 0.014,
                detune_cents: rng.bipolar() * 1.8,
                base_freq: freq,
            });
        }
    }

    /* ---------------- piano partials ---------------- */
    let piano_gain = p.blend * p.p_level;
    let mut piano: Vec<Partial> = Vec::new();
    if piano_gain > 0.002 {
        let harm = p.p_harm.clamp(1.0, 24.0) as i32;
        let base_decay = p.p_decay * (1.0 + 0.9 * ((48.0 - p.midi) / 24.0).exp2());
        let roll = 1.55 - p.p_bright * 0.85;
        let mut sum = 0.0f32;
        let mut tmp: Vec<(f32, f32, f32)> = Vec::new(); // (freq, amp, partial index)
        for n in 1..=harm {
            let nf = n as f32;
            let fnum = nf * f0 * (1.0 + p.p_inharm * nf * nf).sqrt();
            if fnum > sr * 0.46 {
                break;
            }
            let mut a = nf.powf(-roll) * (0.35 + vel * 0.95);
            if nf > 2.0 + vel * 7.0 {
                a *= 0.22;
            }
            tmp.push((fnum, a, nf));
            sum += a;
        }
        let norm = 0.9 / sum.max(0.5);
        for (fnum, a, nf) in tmp {
            let dec = (base_decay / nf.powf(1.45)).max(0.18);
            let tau = (dec / 4.5).max(0.01);
            let detune = rng.bipolar() * 3.2;
            let tuned = fnum * ((detune / 1200.0).exp2());
            piano.push(Partial {
                phase: rng.next_f32(),
                inc: tuned / sr,
                env: 1.0,
                amp: (a * norm * 0.5).max(0.0002),
                decay_mult: (-1.0 / (tau * sr)).exp(),
                attack: 0.005,
                detune_cents: detune,
                base_freq: tuned,
            });
        }
    }

    /* ---------------- transients ---------------- */
    let bright_drawbar = p.draw[2] > 0.0 || p.draw[3] > 0.0 || p.draw[4] > 0.0;
    let chiff_amp = if bright_drawbar { p.chiff * vel * 0.15 * organ_gain } else { 0.0 };
    let hammer_amp = p.p_hammer * vel * 0.17 * piano_gain;
    let mut chiff_filter = Biquad::bandpass(sr, 1100.0 + vel * 1500.0, 0.9);
    let mut hammer_filter = Biquad::bandpass(sr, 1500.0 + vel * 2800.0, 1.1);
    let chiff_mult = (-1.0 / (0.07 * sr)).exp();
    let hammer_mult = (-1.0 / (0.055 * sr)).exp();

    /* ---------------- per-sample loop ---------------- */
    let rot_freq = if p.rotary >= 1.5 {
        26.5
    } else if p.rotary >= 0.5 {
        6.4
    } else {
        0.0
    };
    let leslie = p.leslie.clamp(0.0, 0.95);
    let vib_depth = p.vib_depth;
    let vib_rate = p.vib_rate.max(0.05);
    let bright0 = 900.0 + vel * vel * 8200.0 + p.p_bright * 6500.0;

    let mut lp1 = OnePole::new();
    let mut lp2 = OnePole::new();
    let mut chiff_env = 1.0f32;
    let mut hammer_env = 1.0f32;

    for i in 0..n {
        let t = i as f32 / sr;
        let mut s = 0.0f32;

        /* organ */
        if !organ.is_empty() {
            let vib = vib_depth * (TAU * vib_rate * t).sin();
            let vib_mul = ((vib + 0.0) / 1200.0).exp2();
            for pt in organ.iter_mut() {
                let inc = pt.base_freq * vib_mul / sr;
                pt.phase += inc;
                let env = if t < pt.attack { t / pt.attack } else { 1.0 };
                s += pt.amp * env * (TAU * pt.phase).sin();
            }
            if rot_freq > 0.0 {
                let trem = 1.0 - leslie + leslie * (TAU * rot_freq * t).sin();
                s *= trem;
            }
            s *= organ_gain;
        }

        /* piano */
        if !piano.is_empty() {
            let mut ps = 0.0f32;
            for pt in piano.iter_mut() {
                pt.phase += pt.inc;
                pt.env *= pt.decay_mult;
                let env = if t < pt.attack { t / pt.attack } else { 1.0 };
                ps += pt.amp * pt.env * env * (TAU * pt.phase).sin();
            }
            let fc = bright0 * (0.5 + 0.5 * (-t / 0.9).exp()).max(420.0);
            lp1.set_cutoff(fc, sr);
            lp2.set_cutoff(fc, sr);
            ps = lp2.process(lp1.process(ps));
            s += ps * piano_gain;
        }

        /* chiff (organ air) */
        if chiff_amp > 0.0 {
            let n = rng.bipolar();
            let y = chiff_filter.process(n) * chiff_amp * chiff_env;
            s += y;
            chiff_env *= chiff_mult;
        }
        /* hammer (piano felt) */
        if hammer_amp > 0.0 {
            let n = rng.bipolar();
            let y = hammer_filter.process(n) * hammer_amp * hammer_env;
            s += y;
            hammer_env *= hammer_mult;
        }

        out[i] = s;
    }
}
