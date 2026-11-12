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
    flat: new Float32Array(mem, flatOut, 22)
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
  flatOut = wasm.exports.sonor_alloc(22 * 4);
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
  const a = {
    rms: v.analysis[0], peak: v.analysis[1], centroid: v.analysis[2],
    flatness: v.analysis[3], flux: v.analysis[4], bpm: v.analysis[5],
    confidence: v.analysis[6], root: v.analysisInt[7], scale: v.analysisInt[8],
    clarity: v.analysis[9],
    chroma: Array.from(v.analysis.slice(10, 22)),
    spectrum: Array.from(v.analysis.slice(22, 86))
  };
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
