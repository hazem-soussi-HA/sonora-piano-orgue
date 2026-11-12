/* Sonor machine worker — drives the Rust/WASM core off the main thread.
 *
 *  in : init | render | analyze
 *  out: ready | rendered | analyzed | error
 */
'use strict';

let wasm = null;
let paramsPtr = 0;
let analysisPtr = 0;
let renderBuf = 0;
let renderCap = 0;
let analyzer = 0;
let flatOut = 0;

const PARAM_ORDER = [
  'sample_rate','blend',
  'draw0','draw1','draw2','draw3','draw4','draw5','draw6','draw7',
  'tone','chiff','vib_rate','vib_depth','leslie','rotary','o_level',
  'p_harm','p_decay','p_bright','p_hammer','p_inharm','p_level',
  'midi','vel','seed','reverb_mix','reverb_size','reverb_tone',
  'chorus','chorus_rate','drive','bass','treble','volume'
];

/* the WASM memory can grow on every allocation: views must be rebuilt */
function views() {
  const mem = wasm.exports.memory.buffer;
  return {
    params: new Float32Array(mem, paramsPtr, 40),
    analysis: new Float32Array(mem, analysisPtr, 10 + 12 + 64),
    analysisInt: new Int32Array(mem, analysisPtr, 22),
    flat: new Float32Array(mem, flatOut, 125)
  };
}

async function init(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error('wasm fetch failed: HTTP ' + res.status);
  const bytes = await res.arrayBuffer();
  const { instance } = await WebAssembly.instantiate(bytes, {});
  wasm = instance;
  paramsPtr = wasm.exports.sonor_params_ptr();
  analysisPtr = wasm.exports.sonor_analysis_ptr();
  flatOut = wasm.exports.sonor_alloc(125 * 4);
  analyzer = wasm.exports.analyzer_new(4096);
  return wasm.exports.sonor_abi_version();
}

function writeParams(p) {
  const v = views();
  const q = v.params;
  q[0] = p.sample_rate || 48000;
  q[1] = p.blend;
  for (let i = 0; i < 8; i++) q[2 + i] = (p.draw && p.draw[i]) || 0;
  const rest = PARAM_ORDER.slice(9);
  for (let i = 0; i < rest.length; i++) q[10 + i] = p[rest[i]] !== undefined ? p[rest[i]] : 0;
  q[PARAM_ORDER.indexOf('midi') + 1] = p.midi;
  q[PARAM_ORDER.indexOf('vel') + 1] = p.vel;
  q[PARAM_ORDER.indexOf('seed') + 1] = p.seed;
}

function render(msg) {
  const frames = msg.frames;
  if (frames > renderCap) {
    if (renderBuf) wasm.exports.sonor_free(renderBuf, renderCap);
    renderBuf = wasm.exports.sonor_alloc(frames);
    renderCap = frames;
  }
  writeParams(msg.params);
  wasm.exports.sonor_render(renderBuf, frames);
  const mem = wasm.exports.memory.buffer;
  const pcm = new Float32Array(frames);
  pcm.set(new Float32Array(mem, renderBuf, frames));
  return pcm;
}

function analyze(msg) {
  const n = msg.samples.length;
  const buf = wasm.exports.sonor_alloc(n);
  const mem = wasm.exports.memory.buffer;
  new Float32Array(mem, buf, n).set(msg.samples);
  wasm.exports.analyzer_analyze_flat(analyzer, buf, n, msg.sr, msg.hop, flatOut);
  const v = views();
  /* the flat layout is the documented one: 0..9 scalars, 10..21 chroma,
     22..85 spectrum, 86..92 decoder, 93.. notes, 109.. amplitudes */
  const f = v.flat;
  const a = {
    rms: f[0], peak: f[1], centroid: f[2], flatness: f[3], flux: f[4], bpm: f[5],
    confidence: f[6], root: f[7] | 0, scale: f[8] | 0,
    clarity: f[86],
    chroma: Array.from(f.slice(10, 22)),
    spectrum: Array.from(f.slice(22, 86)),
    onset: f[92],
    note_count: f[88] | 0,
    chord_root: f[89] | 0, chord_quality: f[90] | 0, chord_conf: f[91],
    notes: [], note_amp: []
  };
  for (let i = 0; i < a.note_count && i < 16; i++) {
    a.notes.push(f[93 + i] | 0);
    a.note_amp.push(f[109 + i]);
  }
  wasm.exports.sonor_free(buf, n);
  return a;
}

self.onmessage = async e => {
  const msg = e.data;
  try {
    if (msg.type === 'init') {
      const abi = await init(msg.url);
      self.postMessage({ type: 'ready', abi });
    } else if (msg.type === 'render') {
      const pcm = render(msg);
      self.postMessage({ type: 'rendered', id: msg.id, pcm, frames: msg.frames }, [pcm.buffer]);
    } else if (msg.type === 'analyze') {
      const a = analyze(msg);
      self.postMessage({ type: 'analyzed', id: msg.id, analysis: a });
    }
  } catch (err) {
    self.postMessage({ type: 'error', id: msg.id, message: err.message || String(err) });
  }
};
