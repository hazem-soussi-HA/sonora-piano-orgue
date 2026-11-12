//! Iterative radix-2 FFT (in-place, complex).

/// Complex FFT plan for a fixed size, with cached twiddles and bit-reversal.
pub struct Fft {
    pub(crate) n: usize,
    cos: Vec<f32>,
    sin: Vec<f32>,
    rev: Vec<u32>,
}

impl Fft {
    pub fn new(n: usize) -> Self {
        assert!(n.is_power_of_two(), "fft size must be a power of two");
        let levels = n.trailing_zeros() as usize;
        let mut cos = vec![0.0f32; n / 2];
        let mut sin = vec![0.0f32; n / 2];
        for i in 0..n / 2 {
            let a = -std::f64::consts::PI * 2.0 * i as f64 / n as f64;
            cos[i] = a.cos() as f32;
            sin[i] = a.sin() as f32;
        }
        let mut rev = vec![0u32; n];
        for i in 0..n {
            let mut x = i as u32;
            let mut r = 0u32;
            for _ in 0..levels {
                r = (r << 1) | (x & 1);
                x >>= 1;
            }
            rev[i] = r;
        }
        Fft { n, cos, sin, rev }
    }

    /// In-place complex FFT. `re`/`im` must have length >= n.
    pub fn run(&self, re: &mut [f32], im: &mut [f32]) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i] as usize;
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut size = 2usize;
        while size <= n {
            let half = size / 2;
            let step = n / size;
            let mut i = 0usize;
            while i < n {
                let mut k = 0usize;
                for j in i..i + half {
                    let c = self.cos[k];
                    let s = self.sin[k];
                    let l = j + half;
                    let tre = re[l] * c - im[l] * s;
                    let tim = re[l] * s + im[l] * c;
                    re[l] = re[j] - tre;
                    im[l] = im[j] - tim;
                    re[j] += tre;
                    im[j] += tim;
                    k += step;
                }
                i += size;
            }
            size <<= 1;
        }
    }
}
