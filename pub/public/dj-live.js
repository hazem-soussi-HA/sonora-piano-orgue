/* =====================================================================
   HAZOOM LIVE DJ — a realtime techno engine that collaborates with the
   music it hears.

   The arrangement DNA comes from hazoom-dj-techno (MIT, © 2026 Hazem
   Soussi): the same intro → build → drop → breakdown → drop2 → outro
   structure, the same Am7 voicing and the same minor-key bass motif.

   What is new here is the *collaboration*: the engine listens to the
   system output (the Sonora aether analysis) and locks itself to what
   it hears — the tempo, the key, the brightness and the energy of the
   music — so it plays *with* the record instead of over it.
   ===================================================================== */
'use strict';

const DJ_LIVE = (() => {

  const A = 55;                                   // A1
  const N = m => A * Math.pow(2, m / 12);         // semitone offset
  const CHORD_MIN = [0, 3, 7, 10, 12];            // Am7 voicing (dj-techno)
  const MOTIF = [0, 0, 12, 0, 7, 0, 3, 0, 0, 10, 0, 7, 3, 0, 5, 0];

  const SECTIONS = [
    { kind: 'intro', bars: 8, shift: 0 },
    { kind: 'build', bars: 8, shift: 0 },
    { kind: 'drop', bars: 16, shift: 0 },
    { kind: 'breakdown', bars: 8, shift: -2 },
    { kind: 'drop2', bars: 16, shift: 3 },
    { kind: 'outro', bars: 8, shift: 0 }
  ];

  const SCALES = { major: [0, 2, 4, 5, 7, 9, 11], minor: [0, 2, 3, 5, 7, 8, 10] };

  /* ── voice helpers ───────────────────────────────────────────── */

  function noiseBuffer(ctx, seconds = 2) {
    const len = Math.floor(ctx.sampleRate * seconds);
    const buf = ctx.createBuffer(1, len, ctx.sampleRate);
    const d = buf.getChannelData(0);
    for (let i = 0; i < len; i++) d[i] = Math.random() * 2 - 1;
    return buf;
  }

  function driveCurve(amount) {
    const k = 1 + amount * 22, n = 2048, c = new Float32Array(n), norm = Math.tanh(k) || 1;
    for (let i = 0; i < n; i++) { const x = (i / (n - 1)) * 2 - 1; c[i] = Math.tanh(x * k) / norm; }
    return c;
  }

  function makeIR(ctx, sec, decay) {
    const r = ctx.sampleRate, len = Math.max(1, Math.floor(r * sec));
    const buf = ctx.createBuffer(2, len, r);
    for (let ch = 0; ch < 2; ch++) {
      const d = buf.getChannelData(ch);
      for (let i = 0; i < len; i++) {
        const env = Math.pow(1 - i / len, decay);
        d[i] = (Math.random() * 2 - 1) * env;
      }
    }
    return buf;
  }

  /* ── the engine ──────────────────────────────────────────────── */

  class LiveDJ {
    constructor(ctx, destination) {
      this.ctx = ctx;
      this.out = destination;
      this.playing = false;
      this.step = 0;
      this.nextTime = 0;
      this.timer = null;
      this.bar = 0;

      this.bpm = 132;
      this.root = 9;                 // A
      this.scale = 1;                // minor
      this.intensity = 0.72;         // manual energy 0..1
      this.listen = 0.6;             // how much the engine obeys what it hears
      this.collab = { key: true, tempo: true, filter: true, energy: true };
      this.mood = { bright: 0.5, energy: 0.6, space: 0.5, weight: 0.5 };
      this.heard = { bpm: 0, root: 0, scale: 0, centroid: 0, flux: 0, rms: 0, conf: 0, clarity: 0 };
      this.section = 'intro';
      this.sectionIndex = 0;
      this.level = 0;                // last heard level, for the meter
      this.onStep = null;

      this.build();
    }

    /* ---- graph ---- */
    build() {
      const ctx = this.ctx;
      const n = {};

      n.bus = ctx.createGain();
      n.bus.gain.value = 1;

      n.filter = ctx.createBiquadFilter();
      n.filter.type = 'lowpass';
      n.filter.frequency.value = 12000;
      n.filter.Q.value = 0.9;

      n.drive = ctx.createWaveShaper();
      n.drive.oversample = '2x';
      n.drive.curve = driveCurve(0.18);

      /* sidechain: the kick ducks the melodic layers */
      n.duck = ctx.createGain();
      n.duck.gain.value = 1;

      n.mel = ctx.createGain();       // melodic bus (bass, stabs) — ducked
      n.mel.gain.value = 1;
      n.drums = ctx.createGain();     // drums bus — not ducked
      n.drums.gain.value = 1;

      n.mel.connect(n.duck);
      n.drums.connect(n.bus);
      n.duck.connect(n.bus);
      n.bus.connect(n.filter);
      n.filter.connect(n.drive);

      n.delay = ctx.createDelay(2);
      n.delay.delayTime.value = 0.375;
      n.fb = ctx.createGain(); n.fb.gain.value = 0.36;
      n.delayFilter = ctx.createBiquadFilter();
      n.delayFilter.type = 'highpass'; n.delayFilter.frequency.value = 320;
      n.delaySend = ctx.createGain(); n.delaySend.gain.value = 0.16;
      n.delayReturn = ctx.createGain(); n.delayReturn.gain.value = 0.5;
      n.panL = ctx.createStereoPanner ? ctx.createStereoPanner() : null;
      n.panR = ctx.createStereoPanner ? ctx.createStereoPanner() : null;
      if (n.panL) { n.panL.pan.value = -0.75; n.panR.pan.value = 0.75; }

      n.conv = ctx.createConvolver();
      n.conv.buffer = makeIR(ctx, 2.4, 2.6);
      n.revSend = ctx.createGain(); n.revSend.gain.value = 0.14;
      n.revReturn = ctx.createGain(); n.revReturn.gain.value = 0.7;

      n.master = ctx.createGain(); n.master.gain.value = 0.8;
      n.limiter = ctx.createDynamicsCompressor();
      n.limiter.threshold.value = -6; n.limiter.knee.value = 6;
      n.limiter.ratio.value = 12; n.limiter.attack.value = 0.003; n.limiter.release.value = 0.18;

      n.drive.connect(n.dry = ctx.createGain());
      n.dry.connect(n.master);
      n.drive.connect(n.delaySend);
      n.delaySend.connect(n.delay);
      n.delay.connect(n.delayFilter);
      n.delayFilter.connect(n.fb);
      n.fb.connect(n.delay);
      n.delayFilter.connect(n.delayReturn);
      if (n.panL) { n.delayReturn.connect(n.panL); n.panL.connect(n.master); n.panR.connect(n.master); }
      else n.delayReturn.connect(n.master);
      n.drive.connect(n.revSend);
      n.revSend.connect(n.conv);
      n.conv.connect(n.revReturn);
      n.revReturn.connect(n.master);
      n.master.connect(n.limiter);
      n.limiter.connect(this.out);

      this.N = n;
      this.noise = noiseBuffer(ctx);
    }

    /* ---- transport ---- */
    start() {
      if (this.playing) return;
      this.playing = true;
      this.step = 0; this.bar = 0; this.sectionIndex = 0; this.section = 'intro';
      this.nextTime = this.ctx.currentTime + 0.08;
      clearInterval(this.timer);
      this.timer = setInterval(() => this.tick(), 25);
    }

    stop() {
      this.playing = false;
      clearInterval(this.timer);
      this.timer = null;
      this.allOff();
    }

    allOff() {
      const t = this.ctx.currentTime;
      try { this.N.duck.gain.cancelScheduledValues(t); } catch (e) {}
      this.N.duck.gain.setValueAtTime(this.N.duck.gain.value, t);
      this.N.duck.gain.linearRampToValueAtTime(1, t + 0.05);
    }

    tick() {
      if (!this.playing) return;
      const spb = 60 / this.bpm / 4;         // a sixteenth
      let guard = 0;
      while (this.nextTime < this.ctx.currentTime + 0.15 && guard++ < 64) {
        this.scheduleStep(this.step, this.nextTime);
        this.nextTime += spb;
        this.step++;
      }
    }

    /* ---- the arrangement (dj-techno DNA) ---- */
    sectionFor(bar) {
      let b = 0;
      for (let i = 0; i < SECTIONS.length; i++) {
        if (bar < b + SECTIONS[i].bars) return { ...SECTIONS[i], index: i, barIn: bar - b };
        b += SECTIONS[i].bars;
      }
      return { ...SECTIONS[SECTIONS.length - 1], index: SECTIONS.length - 1, barIn: 0, kind: 'end' };
    }

    density(bar) {
      const s = this.sectionFor(bar);
      const k = s.kind;
      if (k === 'intro') return { kick: true, hat: s.barIn >= 4, bass: s.barIn >= 6, clap: false, stab: false, riser: false };
      if (k === 'build') return { kick: true, hat: true, bass: true, clap: s.barIn >= 4, stab: s.barIn >= 6, riser: s.barIn % 4 === 3 };
      if (k === 'drop') return { kick: true, hat: true, bass: true, clap: true, stab: true, riser: false };
      if (k === 'breakdown') return { kick: s.barIn % 4 === 3, hat: s.barIn >= 4, bass: s.barIn % 2 === 1, clap: false, stab: true, riser: s.barIn % 4 === 3 };
      if (k === 'drop2') return { kick: true, hat: true, bass: true, clap: true, stab: true, riser: false };
      const o = s.barIn;
      return { kick: o < 7, hat: o < 4, bass: o < 6, clap: o < 4, stab: o < 3, riser: false };
    }

    sectionGain(bar) {
      const s = this.sectionFor(bar);
      const k = s.kind;
      if (k === 'intro') return s.barIn < 4 ? 0.40 : 0.58;
      if (k === 'build') return 0.66 + (s.barIn % 8) / 8 * 0.30;
      if (k === 'drop' || k === 'drop2') return 1.0;
      if (k === 'breakdown') return 0.38;
      return 0.95 - Math.min(7, s.barIn) * 0.092;
    }

    /* ---- voices (pure Web Audio, no samples) ---- */
    kick(at, vel = 1) {
      const ctx = this.ctx, N = this.N;
      const o = ctx.createOscillator(); o.type = 'sine';
      o.frequency.setValueAtTime(170, at);
      o.frequency.exponentialRampToValueAtTime(44, at + 0.11);
      const g = ctx.createGain();
      g.gain.setValueAtTime(0.0001, at);
      g.gain.exponentialRampToValueAtTime(0.72 * vel, at + 0.004);
      g.gain.exponentialRampToValueAtTime(0.0001, at + 0.34);
      o.connect(g); g.connect(N.drums);
      o.start(at); o.stop(at + 0.36);

      const s = ctx.createBufferSource(); s.buffer = this.noise; s.loop = true;
      const hp = ctx.createBiquadFilter(); hp.type = 'highpass'; hp.frequency.value = 1200;
      const cg = ctx.createGain();
      cg.gain.setValueAtTime(0.22 * vel, at);
      cg.gain.exponentialRampToValueAtTime(0.0001, at + 0.03);
      s.connect(hp); hp.connect(cg); cg.connect(N.drums);
      s.start(at); s.stop(at + 0.05);

      /* sidechain duck */
      const d = N.duck.gain;
      const depth = 0.45 + 0.45 * this.mood.energy;
      d.cancelScheduledValues(at);
      d.setValueAtTime(1, at);
      d.linearRampToValueAtTime(1 - depth, at + 0.012);
      d.linearRampToValueAtTime(1, at + 0.22);
    }

    bass(at, dur, hz, vel = 1) {
      const ctx = this.ctx, N = this.N;
      const o = ctx.createOscillator(); o.type = 'sawtooth'; o.frequency.value = hz;
      const o2 = ctx.createOscillator(); o2.type = 'square'; o2.frequency.value = hz * 0.5;
      const f = ctx.createBiquadFilter(); f.type = 'lowpass'; f.Q.value = 7;
      f.frequency.setValueAtTime(320 + 2200 * this.mood.bright, at);
      f.frequency.exponentialRampToValueAtTime(180 + 300 * this.mood.bright, at + dur * 0.9);
      const g = ctx.createGain();
      g.gain.setValueAtTime(0.0001, at);
      g.gain.exponentialRampToValueAtTime(0.22 * vel, at + 0.008);
      g.gain.exponentialRampToValueAtTime(0.0001, at + dur);
      o.connect(f); o2.connect(f); f.connect(g); g.connect(N.mel);
      o.start(at); o.stop(at + dur + 0.05);
      o2.start(at); o2.stop(at + dur + 0.05);
    }

    hat(at, open = false, vel = 1) {
      const ctx = this.ctx, N = this.N;
      const s = ctx.createBufferSource(); s.buffer = this.noise; s.loop = true;
      const hp = ctx.createBiquadFilter(); hp.type = 'highpass'; hp.frequency.value = open ? 7000 : 8200;
      const g = ctx.createGain();
      const d = open ? 0.22 : 0.045;
      g.gain.setValueAtTime(0.18 * vel, at);
      g.gain.exponentialRampToValueAtTime(0.0001, at + d);
      s.connect(hp); hp.connect(g); g.connect(N.drums);
      s.start(at); s.stop(at + d + 0.02);
    }

    clap(at, vel = 1) {
      const ctx = this.ctx, N = this.N;
      for (let i = 0; i < 3; i++) {
        const s = ctx.createBufferSource(); s.buffer = this.noise; s.loop = true;
        const bp = ctx.createBiquadFilter(); bp.type = 'bandpass'; bp.frequency.value = 1600; bp.Q.value = 1.4;
        const g = ctx.createGain();
        const t = at + i * 0.011;
        g.gain.setValueAtTime(0.3 * vel, t);
        g.gain.exponentialRampToValueAtTime(0.0001, t + 0.09);
        s.connect(bp); bp.connect(g); g.connect(N.drums);
        s.start(t); s.stop(t + 0.12);
      }
    }

    /* the chord that plays with the record: the heard key, Am7 voiced */
    stab(at, dur, vel = 1) {
      const ctx = this.ctx, N = this.N;
      const base = 220 * Math.pow(2, (this.root - 9) / 12);
      const voicing = CHORD_MIN.map(d => base * Math.pow(2, d / 12));
      const f = ctx.createBiquadFilter(); f.type = 'lowpass'; f.Q.value = 6;
      f.frequency.setValueAtTime(600 + 5200 * this.mood.bright, at);
      f.frequency.exponentialRampToValueAtTime(400 + 900 * this.mood.bright, at + dur);
      const g = ctx.createGain();
      g.gain.setValueAtTime(0.0001, at);
      g.gain.exponentialRampToValueAtTime(0.12 * vel, at + 0.01);
      g.gain.exponentialRampToValueAtTime(0.0001, at + dur);
      f.connect(g); g.connect(N.mel);
      voicing.forEach(hz => {
        for (let d = -1; d <= 1; d++) {
          const o = ctx.createOscillator();
          o.type = 'sawtooth';
          o.frequency.value = hz;
          o.detune.value = d * 7;
          o.connect(f);
          o.start(at); o.stop(at + dur + 0.05);
        }
      });
    }

    riser(at, dur, vel = 1) {
      const ctx = this.ctx, N = this.N;
      const s = ctx.createBufferSource(); s.buffer = this.noise; s.loop = true;
      const bp = ctx.createBiquadFilter(); bp.type = 'bandpass'; bp.Q.value = 3.5;
      bp.frequency.setValueAtTime(400, at);
      bp.frequency.exponentialRampToValueAtTime(9000, at + dur);
      const g = ctx.createGain();
      g.gain.setValueAtTime(0.0001, at);
      g.gain.exponentialRampToValueAtTime(0.14 * vel, at + dur * 0.92);
      g.gain.exponentialRampToValueAtTime(0.0001, at + dur + 0.05);
      s.connect(bp); bp.connect(g); g.connect(N.drums);
      s.start(at); s.stop(at + dur + 0.1);
    }

    impact(at, vel = 1) {
      const ctx = this.ctx, N = this.N;
      const o = ctx.createOscillator(); o.type = 'sine';
      o.frequency.setValueAtTime(120, at);
      o.frequency.exponentialRampToValueAtTime(38, at + 0.5);
      const g = ctx.createGain();
      g.gain.setValueAtTime(0.0001, at);
      g.gain.exponentialRampToValueAtTime(0.34 * vel, at + 0.006);
      g.gain.exponentialRampToValueAtTime(0.0001, at + 0.9);
      o.connect(g); g.connect(N.drums);
      o.start(at); o.stop(at + 1.0);
    }

    /* ---- the collaboration layer ---- */

    /* Feed the analysis of the music the machine is hearing. */
    react(a) {
      if (!a) return;
      const h = this.heard;
      h.bpm = a.bpm || h.bpm;
      h.root = a.root !== undefined ? a.root : h.root;
      h.scale = a.scale !== undefined ? a.scale : h.scale;
      h.centroid = a.centroid || h.centroid;
      h.flux = a.flux || h.flux;
      h.rms = a.rms || h.rms;
      h.conf = a.confidence !== undefined ? a.confidence : h.conf;
      h.clarity = a.clarity !== undefined ? a.clarity : h.clarity;
      this.level = h.rms;

      const t = this.ctx.currentTime;

      /* tempo: follow the record, but never jump more than 8 % at a time */
      if (this.collab.tempo && h.bpm > 50 && h.bpm < 200 && h.clarity > 0.25) {
        const target = h.bpm;
        const limit = this.bpm * 0.08;
        const next = this.bpm + Math.max(-limit, Math.min(limit, target - this.bpm));
        this.bpm += (next - this.bpm) * this.listen * 0.25;
        this.bpm = Math.max(60, Math.min(200, this.bpm));
      }

      /* key: play the chord the record is in */
      if (this.collab.key && h.conf > 0.45) {
        this.root = h.root;
        this.scale = h.scale;
      }

      /* brightness: the filter follows the spectral centroid of the record */
      if (this.collab.filter && h.centroid > 200) {
        const cut = Math.max(700, Math.min(14000, h.centroid * 1.4));
        this.N.filter.frequency.setTargetAtTime(cut, t, 0.25);
      }

      /* energy: the loudness of the record pushes the arrangement */
      if (this.collab.energy) {
        const e = Math.max(0, Math.min(1, h.rms * 2.2));
        this.mood.energy = 0.35 + 0.55 * e;
      }
    }

    /* The healer speaks: map a prompt onto the engine's character. */
    setMoodFromText(text) {
      const s = (text || '').toLowerCase();
      const has = (...w) => w.some(x => s.includes(x));
      const m = this.mood;
      if (has('uplift', 'lift', 'energy', 'rave', 'celebrat', 'euphor', 'transc')) {
        m.bright = 0.85; m.energy = 0.9; m.space = 0.35; m.weight = 0.6;
      } else if (has('calm', 'peace', 'soft', 'gentle', 'healing', 'rest', 'sleep', 'warm')) {
        m.bright = 0.28; m.energy = 0.3; m.space = 0.8; m.weight = 0.45;
      } else if (has('dark', 'deep', 'drone', 'underground', 'shadow', 'abyss')) {
        m.bright = 0.2; m.energy = 0.65; m.space = 0.6; m.weight = 0.9;
      } else if (has('dream', 'ethereal', 'float', 'ghost', 'cosmic', 'void')) {
        m.bright = 0.7; m.energy = 0.45; m.space = 0.95; m.weight = 0.35;
      } else if (has('fracture', 'chaos', 'acid', 'wild', 'raw')) {
        m.bright = 0.95; m.energy = 0.95; m.space = 0.25; m.weight = 0.55;
      }
      this.intensity = 0.25 + 0.65 * m.energy;
      this.N.drive.curve = driveCurve(0.18 + 0.4 * m.weight);
      this.N.revSend.gain.setTargetAtTime(0.06 + 0.22 * m.space, this.ctx.currentTime, 0.4);
      this.N.delaySend.gain.setTargetAtTime(0.08 + 0.2 * m.space, this.ctx.currentTime, 0.4);
      this.N.fb.gain.setTargetAtTime(0.22 + 0.3 * m.space, this.ctx.currentTime, 0.4);
      return m;
    }

    /* ---- step scheduling ---- */
    scheduleStep(step, at) {
      const s16 = step % 16;
      const bar = Math.floor(step / 16);
      const sec = this.sectionFor(bar);
      const d = this.density(bar);
      const gain = this.sectionGain(bar);
      const shift = sec.shift || 0;
      const root = this.root;
      const scale = SCALES[this.scale === 1 ? 'minor' : 'major'];

      if (this.onStep) this.onStep(step, at, sec);

      /* the arrangement section changed: mark it, drop an impact */
      if (sec.index !== this.sectionIndex) {
        this.sectionIndex = sec.index;
        this.section = sec.kind;
        if (sec.kind === 'drop' || sec.kind === 'drop2') this.impact(at, 0.9);
      }

      /* kick on every beat */
      if (d.kick && s16 % 4 === 0) this.kick(at, 0.9 + 0.1 * this.intensity);

      /* clap on 2 and 4 */
      if (d.clap && (s16 === 4 || s16 === 12)) this.clap(at, 0.8);

      /* hats: 8ths in the build, 16ths in the drops */
      const hat16 = (sec.kind === 'drop' || sec.kind === 'drop2') ? s16 % 2 === 0 : s16 % 4 === 2;
      if (d.hat && hat16) this.hat(at, s16 % 8 === 6, 0.55 + 0.35 * this.intensity);

      /* the bass motif — transposed into the key the record is in */
      if (d.bass) {
        const m = MOTIF[s16];
        const deg = scale[((m % 12) + 12) % 12] + 12 * Math.floor(m / 12);
        const hz = N(root + shift + deg + 12);
        if (s16 % 2 === 0 || (sec.kind === 'drop' && s16 % 4 === 3)) {
          this.bass(at, 60 / this.bpm / 4 * 1.7, hz, 0.9);
        }
      }

      /* stabs on the off-beats — the harmony with the record */
      if (d.stab && (s16 === 6 || s16 === 14)) {
        this.stab(at, 60 / this.bpm / 2, 0.8 + 0.3 * this.intensity);
      }

      /* risers into the drop */
      if (d.riser && s16 === 0) {
        this.riser(at, 60 / this.bpm * 1.0, 0.8);
      }

      /* the arrangement level, and the heard energy on top of it */
      const g = gain * (0.6 + 0.6 * this.intensity);
      this.N.master.gain.setTargetAtTime(g * 0.52, at, 0.05);
    }

    /* scale helpers for the UI */
    scaleNotes() {
      const sc = SCALES[this.scale === 1 ? 'minor' : 'major'];
      return sc.map(d => (this.root + d) % 12);
    }

    status() {
      return {
        bpm: this.bpm, root: this.root, scale: this.scale,
        section: this.section, playing: this.playing,
        heard: this.heard, mood: this.mood, intensity: this.intensity
      };
    }
  }

  return { LiveDJ, SECTIONS, SCALES };
})();

if (typeof module !== 'undefined' && module.exports) module.exports = DJ_LIVE;
