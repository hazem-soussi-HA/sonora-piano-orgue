//! Parameter + result blocks exchanged with the host over a flat C ABI.

/// Drawbar harmonic multipliers relative to the 8' fundamental:
/// 16', 8', 4', 2 2/3', 2', 1 1/3', 1 2/3', 1'  ->  1/2, 1, 2, 3/2, 4, 3, 8, 16
pub const DRAW_MULT: [f32; 8] = [0.5, 1.0, 2.0, 1.5, 4.0, 3.0, 8.0, 16.0];

/// Everything the voice renderer needs. Mirrors the JS engine state so both
/// machines can be A/B compared.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SonorParams {
    pub sample_rate: f32,
    pub blend: f32, // 0 = pure organ, 1 = pure piano
    pub draw: [f32; 8],
    pub tone: f32,   // 0 = pure sine .. 1 = reedy
    pub chiff: f32,  // organ attack air
    pub vib_rate: f32,
    pub vib_depth: f32, // cents
    pub leslie: f32,    // tremolo depth
    pub rotary: f32,    // 0 off, 1 slow, 2 fast
    pub o_level: f32,
    pub p_harm: f32,
    pub p_decay: f32,
    pub p_bright: f32,
    pub p_hammer: f32,
    pub p_inharm: f32,
    pub p_level: f32,
    pub midi: f32,
    pub vel: f32,
    pub seed: f32,
    pub reverb_mix: f32,
    pub reverb_size: f32,
    pub reverb_tone: f32,
    pub chorus: f32,
    pub chorus_rate: f32,
    pub drive: f32,
    pub bass: f32,
    pub treble: f32,
    pub volume: f32,
}

impl Default for SonorParams {
    fn default() -> Self {
        SonorParams {
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
        }
    }
}

/// Analysis result block written by the analyzer.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SonorAnalysis {
    pub rms: f32,
    pub peak: f32,
    pub centroid: f32,   // Hz
    pub flatness: f32,   // 0..1
    pub flux: f32,       // spectral flux (onset strength)
    pub bpm: f32,
    pub confidence: f32, // key confidence 0..1
    pub root: i32,       // 0..11 pitch class
    pub scale: i32,      // 0 = major, 1 = minor
    pub clarity: f32,    // tempo clarity 0..1
    pub chroma: [f32; 12],
    pub spectrum: [f32; 64],
    pub frames: i32,
    /// the notes sounding right now: midi numbers and their salience
    pub notes: [f32; 16],
    pub note_amp: [f32; 16],
    pub note_count: i32,
    /// the chord sounding right now
    pub chord_root: i32,
    pub chord_quality: i32, // index into CHORD_NAMES
    pub chord_conf: f32,
    pub onset: f32, // 0..1, the attack of this instant
}

impl Default for SonorAnalysis {
    fn default() -> Self {
        SonorAnalysis {
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
        }
    }
}

/// Chord qualities the decoder can name, in the order of `chord_quality`.
pub const CHORD_NAMES: [&str; 16] = [
    "", "m", "dim", "aug", "sus2", "sus4", "maj7", "m7", "7", "m7b5", "6", "m6", "9", "m9", "add9",
    "maj9",
];

pub const SCALE_MAJOR: i32 = 0;
pub const SCALE_MINOR: i32 = 1;
