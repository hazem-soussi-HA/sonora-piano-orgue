//! Spectral analysis: level, timbre, chroma, key and tempo estimation.

use crate::fast::dot_fast;
use crate::fft::Fft;
use crate::params::{SonorAnalysis, SCALE_MAJOR, SCALE_MINOR};

const MAJOR_PROFILE: [f32; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR_PROFILE: [f32; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

const MAX_FLUX_FRAMES: usize = 512;

pub struct Analyzer {
    fft: Fft,
    re: Vec<f32>,
    im: Vec<f32>,
    mag: Vec<f32>,
    prev_mag: Vec<f32>,
    window: Vec<f32>,
    flux_hist: Vec<f32>,
    frames: i64,
    sr: f32,
    hop: f32,
}

impl Analyzer {
    pub fn new(size: usize) -> Self {
        let fft = Fft::new(size);
        let mut window = vec![0.0f32; size];
        for i in 0..size {
            window[i] = (0.5 - 0.5 * (std::f64::consts::PI * 2.0 * i as f64 / size as f64).cos()) as f32;
        }
        let bins = size / 2;
        Analyzer {
            fft,
            re: vec![0.0; size],
            im: vec![0.0; size],
            mag: vec![0.0; bins],
            prev_mag: vec![0.0; bins],
            window,
            flux_hist: Vec::with_capacity(MAX_FLUX_FRAMES),
            frames: 0,
            sr: 48_000.0,
            hop: 1024.0,
        }
    }

    pub fn analyze(&mut self, data: &[f32], sr: f32, hop: f32, out: &mut SonorAnalysis) {
        let n = self.fft.n;
        self.sr = sr;
        self.hop = hop.max(1.0);
        let take = data.len().min(n);

        /* level */
        let mut sum = 0.0f64;
        let mut peak = 0.0f32;
        for i in 0..take {
            let v = data[i];
            sum += (v as f64) * (v as f64);
            if v.abs() > peak {
                peak = v.abs();
            }
        }
        let rms = (sum / take.max(1) as f64).sqrt() as f32;
        out.rms = rms;
        out.peak = peak;

        /* windowed spectrum */
        for i in 0..n {
            self.re[i] = if i < take { data[i] * self.window[i] } else { 0.0 };
            self.im[i] = 0.0;
        }
        self.fft.run(&mut self.re, &mut self.im);

        let bins = self.mag.len();
        let mut flux = 0.0f32;
        let mut centroid_num = 0.0f64;
        let mut centroid_den = 0.0f64;
        let mut log_sum = 0.0f64;
        let mut arith_sum = 0.0f64;
        let mut count = 0usize;
        for k in 1..bins {
            let re = self.re[k];
            let im = self.im[k];
            let m = (re * re + im * im).sqrt();
            self.mag[k] = m;
            let d = m - self.prev_mag[k];
            if d > 0.0 {
                flux += d;
            }
            self.prev_mag[k] = m;
            let f = k as f32 * sr / n as f32;
            centroid_num += (f as f64) * (m as f64);
            centroid_den += m as f64;
            if m > 1e-7 {
                log_sum += (m as f64).ln();
                arith_sum += m as f64;
                count += 1;
            }
        }
        out.centroid = if centroid_den > 0.0 {
            (centroid_num / centroid_den) as f32
        } else {
            0.0
        };
        out.flatness = if count > 1 && arith_sum > 0.0 {
            let geo = (log_sum / count as f64).exp();
            let arith = arith_sum / count as f64;
            (geo / arith).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        out.flux = flux / bins as f32;

        /* chroma: one peak per semitone.
           Summing energy over a band smears the leakage of a strong partial
           into its neighbours (which then fools the key detector), so each
           semitone claims the loudest bin inside its band instead. */
        let mut chroma = [0.0f32; 12];
        let mut chroma_total = 0.0f32;
        let bin_w = sr / n as f32;
        for m in 33..100 {
            let fc = 440.0 * ((m as f32 - 69.0) / 12.0).exp2();
            if fc < 55.0 || fc > 2200.0 {
                continue;
            }
            let half = (1.0 / 24.0f32).exp2();
            let lo = fc / half;
            let hi = fc * half;
            let k_lo = ((lo / bin_w).ceil() as usize).max(1);
            let k_hi = ((hi / bin_w).floor() as usize).min(bins - 1);
            let mut best_m = 0.0f32;
            for k in k_lo..=k_hi {
                if self.mag[k] > best_m {
                    best_m = self.mag[k];
                }
            }
            if best_m <= 0.0 {
                /* no bin centre inside the band: use the closest one */
                let k = ((fc / bin_w).round() as usize).clamp(1, bins - 1);
                best_m = self.mag[k] * 0.5;
            }
            if best_m > 0.0 {
                chroma[((m % 12) + 12) as usize % 12] += best_m;
                chroma_total += best_m;
            }
        }
        if chroma_total > 0.0 {
            /* Unsharp mask: a partial's spectral skirt still lands in the
               neighbouring semitone, so subtract the local background and
               keep only what stands out. */
            let mut sharp = [0.0f32; 12];
            for i in 0..12 {
                let l = chroma[(i + 11) % 12];
                let r = chroma[(i + 1) % 12];
                sharp[i] = (chroma[i] - 0.5 * (l + r) * 0.5).max(0.0);
            }
            let mut mx = 0.0f32;
            for c in sharp.iter() {
                if *c > mx {
                    mx = *c;
                }
            }
            if mx > 0.0 {
                for c in sharp.iter_mut() {
                    *c /= mx;
                }
            }
            chroma = sharp;
        }
        out.chroma = chroma;

        /* key detection (Krumhansl-Schmuckler) */
        let (root, scale, conf) = detect_key(&chroma);
        out.root = root;
        out.scale = scale;
        out.confidence = conf;

        /* tempo from the flux envelope */
        self.flux_hist.push(flux);
        if self.flux_hist.len() > MAX_FLUX_FRAMES {
            self.flux_hist.remove(0);
        }
        let (bpm, clarity) = self.estimate_tempo();
        out.bpm = bpm;
        out.clarity = clarity;

        /* log-ish spectrum bands for display */
        let mut peak = 0.0f32;
        for k in 1..bins {
            if self.mag[k] > peak {
                peak = self.mag[k];
            }
        }
        let mut band_max = 0.0f32;
        let mut bands = [0.0f32; 64];
        for b in 0..64 {
            let lo = (b * bins) / 64;
            let hi = ((b + 1) * bins) / 64;
            let mut acc = 0.0f32;
            for k in lo..hi.max(lo + 1) {
                acc += self.mag[k];
            }
            bands[b] = if peak > 0.0 { acc / peak } else { 0.0 };
            if bands[b] > band_max {
                band_max = bands[b];
            }
        }
        if band_max > 0.0 {
            for b in 0..64 {
                out.spectrum[b] = (bands[b] / band_max).min(1.0);
            }
        }

        self.frames += 1;
        out.frames = self.frames as i32;
    }

    fn estimate_tempo(&self) -> (f32, f32) {
        let hist_len = self.flux_hist.len();
        if hist_len < 32 {
            return (0.0, 0.0);
        }
        let fps = self.sr / self.hop;
        if fps <= 1.0 {
            return (0.0, 0.0);
        }
        let min_lag = ((fps * 60.0 / 200.0).floor() as usize).max(2);
        let max_lag = ((fps * 60.0 / 50.0).ceil() as usize).min(hist_len / 2);
        if max_lag <= min_lag {
            return (0.0, 0.0);
        }

        /* 1. onset peaks in the flux envelope */
        let mut mean = 0.0f32;
        for v in self.flux_hist.iter() {
            mean += *v;
        }
        mean /= hist_len as f32;
        let mut peaks: Vec<usize> = Vec::new();
        for i in 1..hist_len - 1 {
            let v = self.flux_hist[i];
            if v > self.flux_hist[i - 1] && v >= self.flux_hist[i + 1] && v > mean * 1.15 && v > 1e-6 {
                peaks.push(i);
            }
        }
        if peaks.len() < 3 {
            return (0.0, 0.0);
        }

        /* 2. inter-onset intervals folded into the 50..200 BPM range */
        const BINS: usize = 48;
        let mut hist = [0.0f32; BINS];
        let mut intervals: Vec<f32> = Vec::new();
        for w in peaks.windows(2) {
            let mut dt = (w[1] - w[0]) as f32 * self.hop / self.sr;
            if dt <= 0.0 {
                continue;
            }
            while dt < 0.30 {
                dt *= 2.0;
            }
            while dt > 1.20 {
                dt *= 0.5;
            }
            if dt < 0.30 || dt > 1.20 {
                continue;
            }
            let l2 = dt.log2();
            let bin = (((l2 + 1.737) / 2.0) * BINS as f32) as usize;
            if bin < BINS {
                hist[bin] += 1.0;
                intervals.push(dt);
            }
        }
        let mut best_bin = 0usize;
        let mut best_count = 0.0f32;
        for (i, c) in hist.iter().enumerate() {
            if *c > best_count {
                best_count = *c;
                best_bin = i;
            }
        }
        if best_count < 2.0 {
            return (0.0, 0.0);
        }
        /* refine the interval with the median of the intervals in the winning bin */
        let lo = (-1.737 + 2.0 * best_bin as f32 / BINS as f32).exp2();
        let hi = (-1.737 + 2.0 * (best_bin + 1) as f32 / BINS as f32).exp2();
        let mut in_bin: Vec<f32> = intervals
            .iter()
            .copied()
            .filter(|d| *d >= lo && *d < hi)
            .collect();
        if in_bin.is_empty() {
            return (0.0, 0.0);
        }
        in_bin.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = in_bin[in_bin.len() / 2];
        let mut bpm = 60.0 / median;

        /* 3. refine with the autocorrelation around that period */
        let lag = (fps * 60.0 / bpm).round() as usize;
        let search = ((lag as f32 * 0.12).ceil() as usize).max(2);
        let mut best_r = 0.0f32;
        let mut best_lag = lag;
        for l in lag.saturating_sub(search)..(lag + search).min(hist_len / 2) {
            if l <= min_lag || l >= hist_len {
                continue;
            }
            let a = &self.flux_hist[..hist_len - l];
            let b = &self.flux_hist[l..];
            let r = dot_fast(a, b);
            let na = crate::fast::dot(a, a).sqrt();
            let nb = crate::fast::dot(b, b).sqrt();
            let norm = if na > 1e-9 && nb > 1e-9 { r / (na * nb) } else { 0.0 };
            if norm > best_r {
                best_r = norm;
                best_lag = l;
            }
        }
        if best_r > 0.25 {
            bpm = fps * 60.0 / best_lag as f32;
        }
        let clarity = (best_count / peaks.len() as f32).clamp(0.0, 1.0) * 0.5
            + best_r.clamp(0.0, 1.0) * 0.5;
        (bpm, clarity)
    }
}

/// Pearson correlation of the chroma against every rotated major/minor profile.
fn detect_key(chroma: &[f32; 12]) -> (i32, i32, f32) {
    let mut mean = 0.0f32;
    for c in chroma.iter() {
        mean += *c;
    }
    mean /= 12.0;
    let mut centered = [0.0f32; 12];
    let mut norm = 0.0f32;
    for i in 0..12 {
        centered[i] = chroma[i] - mean;
        norm += centered[i] * centered[i];
    }
    if norm < 1e-9 {
        return (0, SCALE_MAJOR, 0.0);
    }
    let mut best_root = 0i32;
    let mut best_scale = SCALE_MAJOR;
    let mut best_corr = -2.0f32;
    for scale in 0..2 {
        let profile: &[f32; 12] = if scale == 0 { &MAJOR_PROFILE } else { &MINOR_PROFILE };
        let mut pmean = 0.0f32;
        for p in profile.iter() {
            pmean += *p;
        }
        pmean /= 12.0;
        let mut pnorm = 0.0f32;
        for p in profile.iter() {
            let d = *p - pmean;
            pnorm += d * d;
        }
        for root in 0..12 {
            let mut cov = 0.0f32;
            for i in 0..12 {
                /* profile index = scale degree of the pitch class above the root */
                let pi = (i + 12 - root) % 12;
                cov += centered[i] * (profile[pi] - pmean);
            }
            let corr = cov / (norm.sqrt() * pnorm.sqrt()).max(1e-9);
            if corr > best_corr {
                best_corr = corr;
                best_root = root as i32;
                best_scale = scale;
            }
        }
    }
    let confidence = ((best_corr + 1.0) * 0.5).clamp(0.0, 1.0);
    (best_root, best_scale, confidence)
}

/// Scale degrees for a detected key, used by the harmoniser.
pub fn scale_notes(root: i32, scale: i32) -> [i32; 7] {
    if scale == SCALE_MINOR {
        [0, 2, 3, 5, 7, 8, 10]
    } else {
        [0, 2, 4, 5, 7, 9, 11]
    }
    .map(|d| (root + d) % 12)
}
