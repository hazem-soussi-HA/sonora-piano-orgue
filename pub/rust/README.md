# SONOR — the machine in Rust

Two artefacts are built from the same core (`sonor-core`):

| artefact | target | role |
|---|---|---|
| `public/sonor.wasm` | `wasm32-unknown-unknown` | the **voice machine** + the **analyser**, inside the browser |
| `public/sonor-listener.exe` | `x86_64-pc-windows-gnu` | the **listener**: WASAPI loopback of the PC's output device |

```
voice note ─┐
            ├─ sonor-core (Rust) ─► PCM ─► Web Audio master chain (drive, EQ, chorus, reverb)
capture ────┘
   │
   └─ analyser (FFT → chroma → key → tempo) ─► JSON frames ─► node server ─► ws://…/listen ─► every page
```

## Build

```cmd
build.cmd            REM windows: wasm + native listener
```

```bash
./build.sh           # linux/wsl: wasm only
```

The wasm is served by `server.mjs` (`application/wasm`).

Requirements: a Rust toolchain (`https://rustup.rs`) with the `wasm32-unknown-unknown` target,
plus the `stable-x86_64-pc-windows-gnu` toolchain for the native listener
(`rustup toolchain install stable-x86_64-pc-windows-gnu`).
`build.cmd` puts that toolchain's own `rust-lld` on `PATH`, so no external MinGW is needed.

> If an antivirus or application-control policy refuses to run the freshly built helper
> executables, point cargo somewhere else:
> `set CARGO_TARGET_DIR=%TEMP%\sonorbuild` before running `build.cmd`.

## What the core does

**Voice rendering** (`voice.rs`) — additive synthesis, no samples:

* *Orgue*: one oscillator per drawbar, harmonics ½ 1 2 1½ 4 3 8 16 of the 8′ fundamental
  (16′ 8′ 4′ 2⅔′ 2′ 1⅓′ 1⅔′ 1′), tone colour from a regenerated `PeriodicWave`,
  attack *chiff*, vibrato in cents, Leslie tremolo (6.4 Hz / 26.5 Hz).
* *Piano*: inharmonic partials `f(n) = n·f₀·√(1+B·n²)`, per-partial decay `t_n = t₁/n^1.45`,
  velocity-dependent brightness and partial count, hammer-noise transient, body filter.
* A big brass knob crossfades the two engines inside a single note.

**Analysis** (`analyze.rs`):

* radix-2 FFT (hand written, `fft.rs`)
* spectral centroid, flatness, flux
* chroma via a per-semitone peak-picking filterbank + an unsharp mask, so a partial's
  spectral skirt does not pollute the neighbouring pitch class
* key detection by Krumhansl-Schmuckler correlation against rotated major/minor profiles
* tempo from onset peaks → inter-onset-interval histogram → autocorrelation refinement

**Assembly** (`fast.rs`) — the tempo tracker's dot product is a hand-written x86-64 FMA
routine (`vfmadd231ss` on `xmm0/xmm1/xmm2`), selected at runtime when the CPU reports FMA,
with a portable fallback for every other target.

## ABI

Flat C ABI, so no `wasm-bindgen` is needed:

```
sonor_abi_version() -> u32
sonor_params_ptr()  -> *mut SonorParams   // the host writes the patch here
sonor_analysis_ptr()-> *mut SonorAnalysis // the analyser writes its result here
sonor_alloc(frames) -> *mut f32
sonor_free(ptr, frames)
sonor_render(out, frames)                 // renders one note
analyzer_new(size) -> u32
analyzer_analyze(handle, data, n, sr, hop)
analyzer_analyze_flat(handle, data, n, sr, hop, out /* 22 floats */)
sonor_scale_notes(root, scale, out /* 7 i32 */)
```

Note: `root` and `scale` are `i32` inside `SonorAnalysis` — read them through an
`Int32Array` view, not a `Float32Array`.

## The listener

`sonor-listener.exe` opens the default render endpoint a second time as a capture client
with `AUDCLNT_STREAMFLAGS_LOOPBACK` — that is what the speakers are playing, with no
extra driver. It prints one JSON object per analysis window on stdout:

```json
{"rms":0.38,"peak":0.76,"centroid":7897,"flatness":0.65,"flux":2.1,
 "bpm":117.4,"confidence":0.86,"clarity":0.56,"root":6,"scale":0,
 "chroma":[...],"spectrum":[...]}
```

`server.mjs` spawns it, parses the frames and broadcasts them to every browser
connected to `ws://host/listen`. Other backends exist for machines without WASAPI:

```cmd
sonor-listener --list-devices
sonor-listener --file track.wav
sonor-listener --backend ffmpeg --device "virtual-audio-capturer"
```

## Tests

```cmd
node test-core.js    REM renders voices + analyses synthetic audio, prints the spectra
```
