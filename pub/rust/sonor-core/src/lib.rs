//! Sonor machine core.
//!
//! The same library is compiled twice:
//!   * `wasm32-unknown-unknown` — the in-browser engine (voice rendering + analysis)
//!   * `x86_64-pc-windows-gnu`  — the native listener that captures the PC's audio
//!
//! Everything is exposed through a flat C ABI so no wasm-bindgen is needed and
//! the host can drive it with plain typed arrays.

pub mod analyze;
pub mod fast;
pub mod fft;
pub mod params;
pub mod voice;

use analyze::Analyzer;
use params::{SonorAnalysis, SonorParams};
use std::cell::UnsafeCell;

/// Single-threaded cell used to park the shared parameter/analysis blocks.
struct SyncCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for SyncCell<T> {}

static PARAMS: SyncCell<SonorParams> = SyncCell(UnsafeCell::new(SonorParams {
    sample_rate: 48_000.0,
    blend: 0.5,
    draw: [0.0, 8.0, 8.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    tone: 0.3,
    chiff: 0.5,
    vib_rate: 5.4,
    vib_depth: 5.0,
    leslie: 0.35,
    rotary: 1.0,
    o_level: 0.85,
    p_harm: 10.0,
    p_decay: 7.0,
    p_bright: 0.55,
    p_hammer: 0.5,
    p_inharm: 0.0004,
    p_level: 0.8,
    midi: 60.0,
    vel: 0.9,
    seed: 1.0,
    reverb_mix: 0.28,
    reverb_size: 2.6,
    reverb_tone: 0.5,
    chorus: 0.3,
    chorus_rate: 0.5,
    drive: 0.12,
    bass: 0.0,
    treble: 2.0,
    volume: 0.8,
}));

static ANALYSIS: SyncCell<SonorAnalysis> = SyncCell(UnsafeCell::new(SonorAnalysis {
    rms: 0.0,
    peak: 0.0,
    centroid: 0.0,
    flatness: 0.0,
    flux: 0.0,
    bpm: 0.0,
    confidence: 0.0,
    root: 0,
    scale: 0,
    clarity: 0.0,
    chroma: [0.0; 12],
    spectrum: [0.0; 64],
    frames: 0,
    notes: [0.0; 16],
    note_amp: [0.0; 16],
    note_count: 0,
    chord_root: 0,
    chord_quality: 0,
    chord_conf: 0.0,
    onset: 0.0,
}));

/// Analyzer instances live in a small registry so the host can keep several
/// independent streams (one per capture source).
struct Registry {
    items: Vec<Option<Analyzer>>,
}
static REGISTRY: SyncCell<Registry> = SyncCell(UnsafeCell::new(Registry { items: Vec::new() }));

/// Access the analyzer registry. The machine is single threaded by design:
/// the browser drives one WASM instance, the listener drives one native one.
fn registry() -> &'static mut Registry {
    // SAFETY: no other thread ever touches the registry.
    unsafe { &mut *REGISTRY.0.get() }
}

/// ABI version — bumped when the struct layout changes.
#[no_mangle]
pub extern "C" fn sonor_abi_version() -> u32 {
    4
}

/// Pointer to the parameter block the host writes before rendering.
#[no_mangle]
pub extern "C" fn sonor_params_ptr() -> *mut SonorParams {
    PARAMS.0.get()
}

/// Pointer to the analysis block the host reads after analysing.
#[no_mangle]
pub extern "C" fn sonor_analysis_ptr() -> *mut SonorAnalysis {
    ANALYSIS.0.get()
}

/// Allocate a sample buffer inside the machine's heap.
///
/// # Safety
/// The returned pointer must be released with `sonor_free`.
#[no_mangle]
pub extern "C" fn sonor_alloc(frames: usize) -> *mut f32 {
    let mut v = vec![0.0f32; frames];
    let ptr = v.as_mut_ptr();
    std::mem::forget(v);
    ptr
}

/// Release a buffer obtained from `sonor_alloc`.
///
/// # Safety
/// `ptr` must come from `sonor_alloc` with the same `frames`.
#[no_mangle]
pub unsafe extern "C" fn sonor_free(ptr: *mut f32, frames: usize) {
    if ptr.is_null() || frames == 0 {
        return;
    }
    drop(Vec::from_raw_parts(ptr, frames, frames));
}

/// Render one note into a host allocated buffer.
///
/// # Safety
/// `out` must point to at least `frames` writable f32 values.
#[no_mangle]
pub unsafe extern "C" fn sonor_render(out: *mut f32, frames: usize) -> i32 {
    if out.is_null() || frames == 0 {
        return -1;
    }
    let params = &*PARAMS.0.get();
    let slice = std::slice::from_raw_parts_mut(out, frames);
    voice::render_note(slice, params);
    0
}

/// Create an analyzer instance. Returns a handle (> 0) or 0 on failure.
#[no_mangle]
pub extern "C" fn analyzer_new(size: usize) -> u32 {
    let size = size.next_power_of_two().clamp(256, 8192);
    let items = &mut registry().items;
    if let Some(idx) = items.iter().position(|x| x.is_none()) {
        items[idx] = Some(Analyzer::new(size));
        return (idx + 1) as u32;
    }
    items.push(Some(Analyzer::new(size)));
    (items.len()) as u32
}

/// Destroy an analyzer instance.
#[no_mangle]
pub extern "C" fn analyzer_free(handle: u32) {
    if handle == 0 {
        return;
    }
    let items = &mut registry().items;
    let idx = (handle - 1) as usize;
    if idx < items.len() {
        items[idx] = None;
    }
}

/// Analyse a block of mono samples. Results land in the analysis block.
///
/// # Safety
/// `data` must point to at least `n` readable f32 values.
#[no_mangle]
pub unsafe extern "C" fn analyzer_analyze(handle: u32, data: *const f32, n: usize, sr: f32, hop: f32) -> i32 {
    if data.is_null() || n == 0 || handle == 0 {
        return -1;
    }
    let reg = registry();
    let items = &mut reg.items;
    let idx = (handle - 1) as usize;
    let analyzer = match items.get_mut(idx).and_then(|x| x.as_mut()) {
        Some(a) => a,
        None => return -2,
    };
    let slice = std::slice::from_raw_parts(data, n);
    let mut out = &mut *ANALYSIS.0.get();
    analyzer.analyze(slice, sr, hop, &mut out);
    0
}

/// Convenience: analyse and return the results as a flat f32 array.
///
/// Layout (125 floats):
///   0..9    rms, peak, centroid, flatness, flux, bpm, confidence, root, scale
///   10..21  chroma[12]
///   22..85  spectrum[64]
///   86..91  clarity, frames, note_count, chord_root, chord_quality, chord_conf
///   92      onset
///   93..108 notes[16]      (midi numbers)
///   109..124 note_amp[16]  (salience)
///
/// # Safety
/// `out` must point to at least 125 writable f32 values.
#[no_mangle]
pub unsafe extern "C" fn analyzer_analyze_flat(
    handle: u32,
    data: *const f32,
    n: usize,
    sr: f32,
    hop: f32,
    out: *mut f32,
) -> i32 {
    let rc = analyzer_analyze(handle, data, n, sr, hop);
    if rc != 0 {
        return rc;
    }
    let a = &*ANALYSIS.0.get();
    let o = std::slice::from_raw_parts_mut(out, 125);
    o[0] = a.rms;
    o[1] = a.peak;
    o[2] = a.centroid;
    o[3] = a.flatness;
    o[4] = a.flux;
    o[5] = a.bpm;
    o[6] = a.confidence;
    o[7] = a.root as f32;
    o[8] = a.scale as f32;
    o[9] = a.clarity;
    for i in 0..12 {
        o[10 + i] = a.chroma[i];
    }
    for i in 0..64 {
        o[22 + i] = a.spectrum[i];
    }
    o[86] = a.clarity;
    o[87] = a.frames as f32;
    o[88] = a.note_count as f32;
    o[89] = a.chord_root as f32;
    o[90] = a.chord_quality as f32;
    o[91] = a.chord_conf;
    o[92] = a.onset;
    for i in 0..16 {
        o[93 + i] = a.notes[i];
        o[109 + i] = a.note_amp[i];
    }
    0
}

/// The names of the chord qualities, as a NUL separated string.
/// The caller must not free it.
#[no_mangle]
pub extern "C" fn sonor_chord_names_ptr() -> *const u8 {
    crate::params::CHORD_NAMES.join("\n").as_ptr()
}

/// Scale degrees of the detected key, written into `out` (7 entries).
///
/// # Safety
/// `out` must point to at least 7 writable i32 values.
#[no_mangle]
pub unsafe extern "C" fn sonor_scale_notes(root: i32, scale: i32, out: *mut i32) {
    if out.is_null() {
        return;
    }
    let notes = analyze::scale_notes(root, scale);
    let o = std::slice::from_raw_parts_mut(out, 7);
    o.copy_from_slice(&notes);
}
