import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import url from 'node:url';
import { spawn } from 'node:child_process';

const PORT = process.env.PORT || 8787;
const CANDIDATES = [path.join(process.cwd(), 'public'), path.join(process.cwd(), 'pub', 'public')];
const ROOT = CANDIDATES.find(p => fs.existsSync(p)) || CANDIDATES[0];
const AUDIO = path.join(ROOT, 'audio');
if (!fs.existsSync(AUDIO)) fs.mkdirSync(AUDIO, { recursive: true });

const MIME = {'.html':'text/html','.js':'text/javascript','.css':'text/css','.mp3':'audio/mpeg','.wav':'audio/wav','.ogg':'audio/ogg','.m4a':'audio/mp4','.flac':'audio/flac','.json':'application/json','.txt':'text/plain','.wasm':'application/wasm','.exe':'application/octet-stream'};
const CORS = {'Access-Control-Allow-Origin':'*','Access-Control-Allow-Methods':'GET,POST,OPTIONS','Access-Control-Allow-Headers':'Content-Type,Range','Access-Control-Expose-Headers':'Content-Length,Content-Range,Accept-Ranges'};
let STATE = {};

const send = (res, code, body, headers={}) => { res.writeHead(code, {...CORS, ...headers}); res.end(body); };

function serveFile(req, res, file) {
  fs.stat(file, (err, st) => {
    if (err || !st.isFile()) return send(res, 404, 'not found');
    const ct = MIME[path.extname(file).toLowerCase()] || 'application/octet-stream';
    const rg = req.headers.range;
    if (rg) {
      const m = /bytes=(\d*)-(\d*)/.exec(rg);
      if (!m) return send(res, 416, '');
      const start = m[1] ? +m[1] : 0;
      const end = m[2] ? +m[2] : st.size - 1;
      if (start >= st.size || end >= st.size) return send(res, 416, '', {'Content-Range':`bytes */${st.size}`});
      res.writeHead(206, {...CORS, 'Content-Type':ct, 'Content-Length':end-start+1, 'Content-Range':`bytes ${start}-${end}/${st.size}`, 'Accept-Ranges':'bytes'});
      fs.createStream(file, {start, end}).pipe(res);
    } else {
      res.writeHead(200, {...CORS, 'Content-Type':ct, 'Content-Length':st.size, 'Accept-Ranges':'bytes'});
      fs.createReadStream(file).pipe(res);
    }
  });
}

/* ------------------------------------------------------------------ *
 *  AETHER HUB — the server listens to the machine room.
 *  Every client on /listen receives the analysis frames produced by the
 *  native Rust listener (WASAPI loopback of the PC's output device).
 * ------------------------------------------------------------------ */
const WS_GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';
const clients = new Set();
let lastFrame = null;
let listenerState = { running: false, error: null, started: null, frames: 0 };

function wsAccept(key) {
  return crypto.createHash('sha1').update(key + WS_GUID).digest('base64');
}
function wsFrame(str) {
  const payload = Buffer.from(str, 'utf8');
  const len = payload.length;
  let header;
  if (len < 126) { header = Buffer.alloc(2); header[1] = len; }
  else if (len < 65536) { header = Buffer.alloc(4); header[1] = 126; header.writeUInt16BE(len, 2); }
  else { header = Buffer.alloc(10); header[1] = 127; header.writeBigUInt64BE(BigInt(len), 2); }
  header[0] = 0x81; // FIN + text
  return Buffer.concat([header, payload]);
}
function wsParse(buf) {
  if (buf.length < 2) return null;
  const opcode = buf[0] & 0x0f;
  const masked = (buf[1] & 0x80) !== 0;
  let len = buf[1] & 0x7f;
  let off = 2;
  if (len === 126) { if (buf.length < 4) return null; len = buf.readUInt16BE(2); off = 4; }
  else if (len === 127) { if (buf.length < 10) return null; len = Number(buf.readBigUInt64BE(2)); off = 10; }
  let mask = null;
  if (masked) { if (buf.length < off + 4) return null; mask = buf.subarray(off, off + 4); off += 4; }
  if (buf.length < off + len) return null;
  const data = Buffer.from(buf.subarray(off, off + len));
  if (mask) for (let i = 0; i < data.length; i++) data[i] ^= mask[i % 4];
  return { opcode, data, rest: buf.subarray(off + len) };
}
function broadcast(obj) {
  const frame = wsFrame(JSON.stringify(obj));
  for (const c of clients) { try { c.socket.write(frame); } catch (e) { clients.delete(c); } }
}
function handleUpgrade(req, socket) {
  const key = req.headers['sec-websocket-key'];
  if (!key) { socket.destroy(); return; }
  socket.write('HTTP/1.1 101 Switching Protocols\r\n' +
    'Upgrade: websocket\r\n' +
    'Connection: Upgrade\r\n' +
    'Sec-WebSocket-Accept: ' + wsAccept(key) + '\r\n\r\n');
  socket.setNoDelay(true);
  const client = { socket, alive: true };
  clients.add(client);
  socket.write(wsFrame(JSON.stringify({ type: 'hello', clients: clients.size, listener: listenerState })));
  let buf = Buffer.alloc(0);
  socket.on('data', chunk => {
    buf = Buffer.concat([buf, chunk]);
    let msg;
    while ((msg = wsParse(buf)) !== null) {
      buf = msg.rest;
      if (msg.opcode === 8) { socket.end(); clients.delete(client); return; }
      if (msg.opcode === 9) { socket.write(Buffer.from([0x8a, 0x80, 0, 0, 0, 0])); continue; }
      if (msg.opcode === 1) {
        let obj = null;
        try { obj = JSON.parse(msg.data.toString('utf8')); } catch (e) {}
        if (obj) {
          if (obj.type === 'publish' && obj.frame) {
            lastFrame = obj.frame;
            broadcast({ type: 'frame', source: obj.source || 'browser', frame: obj.frame });
          } else if (obj.type === 'ping') {
            socket.write(wsFrame(JSON.stringify({ type: 'pong', t: Date.now() })));
          }
        }
      }
    }
  });
  socket.on('close', () => clients.delete(client));
  socket.on('error', () => clients.delete(client));
}

/* ---- native listener (Rust, WASAPI loopback) ---- */
function findListener() {
  const candidates = [
    process.env.SONOR_LISTENER,
    path.join(ROOT, 'sonor-listener.exe'),
    path.join(ROOT, 'bin', 'sonor-listener.exe'),
    path.join(process.cwd(), 'pub', 'rust', 'target', 'x86_64-pc-windows-gnu', 'release', 'sonor-listener.exe'),
  ].filter(Boolean);
  for (const c of candidates) { try { if (fs.existsSync(c)) return c; } catch (e) {} }
  return null;
}
function startListener() {
  const exe = findListener();
  if (!exe) {
    listenerState = { running: false, error: 'sonor-listener.exe not found', started: null, frames: 0 };
    return;
  }
  try {
    const child = spawn(exe, [], { windowsHide: true });
    let carry = '';
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => {
      carry += chunk;
      let idx;
      while ((idx = carry.indexOf('\n')) >= 0) {
        const line = carry.slice(0, idx).trim();
        carry = carry.slice(idx + 1);
        if (!line) continue;
        try {
          const frame = JSON.parse(line);
          lastFrame = frame;
          listenerState.frames++;
          broadcast({ type: 'frame', source: 'native', frame });
        } catch (e) { /* ignore malformed lines */ }
      }
    });
    child.stderr.setEncoding('utf8');
    child.stderr.on('data', d => process.stderr.write('[sonor-listener] ' + d));
    child.on('exit', code => {
      listenerState = { running: false, error: 'listener exited (' + code + ')', started: listenerState.started, frames: listenerState.frames };
      broadcast({ type: 'listener', state: listenerState });
    });
    child.on('error', e => {
      listenerState = { running: false, error: e.message, started: null, frames: 0 };
      broadcast({ type: 'listener', state: listenerState });
    });
    listenerState = { running: true, error: null, started: Date.now(), frames: 0 };
    console.log('⚗  aether listener -> ' + exe);
  } catch (e) {
    listenerState = { running: false, error: e.message, started: null, frames: 0 };
  }
}

/* ------------------------------------------------------------------ *
 *  THE HEALER — the private nano-protocol service plus the model.
 *  The doctor content stays in its own private repository; the studio
 *  talks to it over localhost and the model bridge turns the musical
 *  state into an inspirational direction.
 * ------------------------------------------------------------------ */
const HEALER_URL = process.env.HAZOOM_HEALER_URL || 'http://127.0.0.1:8098';
const MODEL_URL = process.env.HAZOOM_MODEL_URL || '';
const MODEL_KEY = process.env.HAZOOM_MODEL_KEY || '';
const MODEL_NAME = process.env.HAZOOM_MODEL_NAME || 'local-model';

async function healerService(path) {
  const r = await fetch(HEALER_URL + path, { headers: { 'Accept': 'application/json' } });
  if (!r.ok) throw new Error('doctor service: HTTP ' + r.status);
  return r.json();
}

/* A deterministic direction generator — used when no model is configured,
   so the studio always has something to say. */
function localDirection(ctx) {
  const names = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
  const key = names[ctx.root] + ' ' + (ctx.scale ? 'minor' : 'major');
  const heard = ctx.heardBpm ? `the record runs at ${ctx.heardBpm.toFixed(0)} BPM` : 'no record is playing yet';
  const anchors = [
    [174, 'foundation', 'ground the low end'],
    [285, 'tissue repair', 'let the pads breathe'],
    [417, 'change', 'open the filter on the transition'],
    [528, 'transformation', 'put the choir on the drop'],
    [639, 'connection', 'widen the stereo field'],
    [741, 'intuition', 'strip back and leave space'],
    [852, 'spiritual order', 'hold the pattern, do not add']
  ];
  const anchor = anchors[Math.abs(Math.round(ctx.root * 3 + ctx.bpm / 40)) % anchors.length];
  const mood = ctx.intention ? ctx.intention.trim() : 'open the sky';
  const dir = [
    `Hold the sub and let the silence do the work; ${heard}.`,
    `Play the ${key} voicing under the record so the two never fight.`,
    `Open the filter from ${(600 + ctx.bright * 9000) | 0} Hz across eight bars and let the riser arrive one bar before the drop.`,
    `Sidechain deep on every kick, but keep the clap dry.`,
    `Anchor the set on ${anchor[0]} Hz — ${anchor[1]}, ${anchor[2]}.`,
    `The intention is: ${mood}. Answer it with dynamics, not with more layers.`
  ];
  return dir.join(' ');
}

async function modelDirection(ctx) {
  const system = [
    'You are the sound healer of a live techno studio.',
    'You write short, vivid DJ directions that fuse the listener\'s intention',
    'with the music the machine is hearing (its key, tempo and energy).',
    'Speak in imperative sentences, never more than six, and always name one',
    'concrete action: a filter move, a transition, a frequency to anchor on.',
    'Never claim medical effects.'
  ].join(' ');
  const user = [
    'Intentions: ' + (ctx.intention || '(none given)'),
    'Intention category: ' + (ctx.category || 'unspecified'),
    'The record the studio is hearing: ' + (ctx.heardBpm ? ctx.heardBpm.toFixed(1) + ' BPM, key ' + ctx.heardKey : 'silence'),
    'The engine is playing: ' + ctx.key + ' at ' + ctx.bpm.toFixed(1) + ' BPM, section ' + ctx.section,
    'Healing anchor available: ' + ctx.anchor,
    'Write the direction.'
  ].join('\n');

  const body = {
    model: MODEL_NAME,
    messages: [{ role: 'system', content: system }, { role: 'user', content: user }],
    temperature: 0.85,
    max_tokens: 320
  };
  const headers = { 'Content-Type': 'application/json' };
  if (MODEL_KEY) headers['Authorization'] = 'Bearer ' + MODEL_KEY;
  const r = await fetch(MODEL_URL, { method: 'POST', headers, body: JSON.stringify(body) });
  if (!r.ok) throw new Error('model: HTTP ' + r.status + ' ' + (await r.text()).slice(0, 200));
  const j = await r.json();
  const text = (j.choices && j.choices[0] && j.choices[0].message && j.choices[0].message.content) ||
    (j.response || j.text || '');
  return String(text).trim();
}

/* ------------------------------------------------------------------ */
const server = http.createServer(async (req, res) => {
  const u = url.parse(req.url, true);
  console.log(`[${req.method}] ${u.pathname}`);
  if (req.method === 'OPTIONS') return send(res, 204, '');

  if (u.pathname === '/api/tracks') {
    try {
      const files = await fs.promises.readdir(AUDIO);
      const tracks = [];
      for (const f of files) {
        if (!/\.(mp3|wav|ogg|m4a|flac)$/i.test(f)) continue;
        const st = await fs.promises.stat(path.join(AUDIO, f));
        tracks.push({name:f, url:'/media/'+encodeURIComponent(f), size:st.size, mtime:st.mtimeMs});
      }
      return send(res, 200, JSON.stringify(tracks), {'Content-Type':'application/json','Cache-Control':'no-store'});
    } catch (e) { return send(res, 500, JSON.stringify({error:e.message}), {'Content-Type':'application/json'}); }
  }

  if (u.pathname === '/api/state') {
    if (req.method === 'GET') return send(res, 200, JSON.stringify(STATE), {'Content-Type':'application/json','Cache-Control':'no-store'});
    if (req.method === 'POST') {
      let body = '';
      req.on('data', c => { body += c; if (body.length > 8192) req.destroy(); });
      req.on('end', () => {
        try {
          const j = JSON.parse(body);
          const keys = ['macro','filter','q','bass','drive','crush','pump','delayTime','delayFB','delayMix','reverbSize','reverbMix','volume','rate','stage','play'];
          const clean = {}; for (const k of keys) if (k in j) clean[k] = j[k];
          STATE = clean;
          send(res, 200, JSON.stringify(STATE), {'Content-Type':'application/json','Cache-Control':'no-store'});
        } catch { send(res, 400, '{"error":"invalid json"}', {'Content-Type':'application/json'}); }
      });
      return;
    }
  }

  /* latest aether analysis frame */
  if (u.pathname === '/api/listen') {
    return send(res, 200, JSON.stringify({ listener: listenerState, clients: clients.size, frame: lastFrame }),
      { 'Content-Type':'application/json', 'Cache-Control':'no-store' });
  }

  /* the doctor service (private nano-protocol) behind a local proxy
     (the /prompt route below is handled first) */
  if (u.pathname.startsWith('/api/healer/') && u.pathname !== '/api/healer/prompt') {
    const rest = u.pathname.slice('/api/healer'.length) || '/';
    try {
      const data = await healerService(rest);
      return send(res, 200, JSON.stringify(data), { 'Content-Type':'application/json', 'Cache-Control':'no-store' });
    } catch (e) {
      return send(res, 503, JSON.stringify({ error: e.message, service: HEALER_URL }),
        { 'Content-Type':'application/json' });
    }
  }

  /* the intelligent sound healer: intention + musical state -> direction */
  if (u.pathname === '/api/healer/prompt' && req.method === 'POST') {
    let body = '';
    req.on('data', c => { body += c; if (body.length > 8192) req.destroy(); });
    req.on('end', async () => {
      let input = {};
      try { input = JSON.parse(body || '{}'); } catch (e) {}
      const f = lastFrame || {};
      const names = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
      const anchorMap = {
        174: 'foundation', 285: 'tissue repair', 396: 'liberating fear', 417: 'change',
        528: 'transformation', 639: 'connection', 741: 'intuition', 852: 'spiritual order', 963: 'divine consciousness'
      };
      let anchor = '528 Hz — transformation, love, miracles';
      try {
        const mods = await healerService('/api/frequencies');
        const list = mods && mods.frequencies ? mods.frequencies : [];
        if (list.length) {
          const pick = list[Math.abs(Math.round((f.root || 0) * 3)) % list.length];
          const hz = pick.hz || pick.frequency;
          if (hz) anchor = hz + ' Hz — ' + (anchorMap[hz] || pick.name || 'intention');
        }
      } catch (e) { /* the doctor service is optional */ }

      const ctx = {
        intention: input.intention || '',
        category: input.category || 'unspecified',
        root: input.root !== undefined ? input.root : (f.root || 0),
        scale: input.scale !== undefined ? input.scale : (f.scale || 0),
        bpm: input.bpm || 132,
        section: input.section || 'intro',
        intensity: input.intensity !== undefined ? input.intensity : 0.7,
        bright: input.bright !== undefined ? input.bright : 0.5,
        heardBpm: f.bpm || 0,
        heardKey: f.root !== undefined ? names[f.root] + ' ' + (f.scale ? 'min' : 'maj') : '',
        key: names[input.root !== undefined ? input.root : (f.root || 0)],
        anchor
      };

      let prompt = null, provider = 'local';
      if (MODEL_URL) {
        try { prompt = await modelDirection(ctx); provider = MODEL_NAME; }
        catch (e) { prompt = localDirection(ctx); provider = 'local (model failed: ' + e.message + ')'; }
      } else {
        prompt = localDirection(ctx);
      }
      return send(res, 200, JSON.stringify({ prompt, provider, model: MODEL_NAME || null, context: ctx }),
        { 'Content-Type':'application/json', 'Cache-Control':'no-store' });
    });
    return;
  }

  if (u.pathname === '/api/proxy' && req.method === 'GET') {
    const target = u.query.url;
    if (!target || !/^https?:\/\//i.test(target)) return send(res, 400, 'bad url');
    try {
      const fh = req.headers.range ? {Range: req.headers.range} : {};
      const r = await fetch(target, {headers: fh});
      const buf = Buffer.from(await r.arrayBuffer());
      const h = {...CORS, 'Content-Type':r.headers.get('content-type')||'audio/mpeg', 'Content-Length':buf.length, 'Accept-Ranges':'bytes'};
      const cr = r.headers.get('content-range'); if (cr) h['Content-Range'] = cr;
      res.writeHead(r.status, h); res.end(buf);
    } catch (e) { send(res, 502, 'proxy failed: '+e.message); }
    return;
  }

  if (u.pathname.startsWith('/media/')) {
    const name = decodeURIComponent(u.pathname.slice(7));
    if (name.includes('..') || /[\/\\]/.test(name)) return send(res, 400, 'bad');
    const file = path.join(AUDIO, name);
    if (!file.startsWith(AUDIO)) return send(res, 400, 'bad');
    return serveFile(req, res, file);
  }

  const p = u.pathname === '/' ? '/index.html' : u.pathname;
  const file = path.join(ROOT, p);
  if (!file.startsWith(ROOT)) return send(res, 403, 'forbidden');
  serveFile(req, res, file);
});

server.on('upgrade', (req, socket, head) => {
  const u = url.parse(req.url, true);
  if (u.pathname === '/listen') return handleUpgrade(req, socket);
  socket.destroy();
});

server.listen(PORT, () => {
  console.log(`⚗  http://localhost:${PORT}`);
  startListener();
});
