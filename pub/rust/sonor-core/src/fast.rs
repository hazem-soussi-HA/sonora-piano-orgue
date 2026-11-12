//! Low level numeric kernels.
//!
//! On x86-64 the dot product used by the tempo tracker is executed by a
//! hand-written FMA assembly routine; every other target uses a portable
//! implementation that the auto-vectorizer can handle.

#[cfg(target_arch = "x86_64")]
pub mod x86 {
    use core::arch::asm;

    /// Hand-written FMA dot product.
    ///
    /// sum = a[0]*b[0] + a[1]*b[1] + ... (n elements)
    ///
    /// The accumulator lives in xmm0 and the two operands are staged through
    /// xmm1/xmm2, so the loop body is four fused multiply-adds.
    #[target_feature(enable = "fma")]
    pub unsafe fn dot_fma(a: *const f32, b: *const f32, n: usize) -> f32 {
        let mut acc: f32 = 0.0;
        let mut i = 0usize;
        while i + 4 <= n {
            asm!(
                "vmovss  xmm1, [{a}]",
                "vmovss  xmm2, [{b}]",
                "vfmadd231ss xmm0, xmm1, xmm2",
                "vmovss  xmm1, [{a4}]",
                "vmovss  xmm2, [{b4}]",
                "vfmadd231ss xmm0, xmm1, xmm2",
                "vmovss  xmm1, [{a8}]",
                "vmovss  xmm2, [{b8}]",
                "vfmadd231ss xmm0, xmm1, xmm2",
                "vmovss  xmm1, [{a12}]",
                "vmovss  xmm2, [{b12}]",
                "vfmadd231ss xmm0, xmm1, xmm2",
                a = in(reg) a.add(i),
                b = in(reg) b.add(i),
                a4 = in(reg) a.add(i + 1),
                b4 = in(reg) b.add(i + 1),
                a8 = in(reg) a.add(i + 2),
                b8 = in(reg) b.add(i + 2),
                a12 = in(reg) a.add(i + 3),
                b12 = in(reg) b.add(i + 3),
                inout("xmm0") acc,
                out("xmm1") _,
                out("xmm2") _,
                options(nostack, readonly),
            );
            i += 4;
        }
        while i < n {
            acc += *a.add(i) * *b.add(i);
            i += 1;
        }
        acc
    }

    /// True when the CPU supports the FMA + SSE2 path used above.
    pub fn has_fma() -> bool {
        #[cfg(target_feature = "fma")]
        {
            true
        }
        #[cfg(not(target_feature = "fma"))]
        {
            std::is_x86_feature_detected!("fma")
        }
    }
}

/// Portable dot product.
#[inline]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut sum = 0.0f32;
    let mut i = 0usize;
    while i + 4 <= n {
        sum += a[i] * b[i] + a[i + 1] * b[i + 1] + a[i + 2] * b[i + 2] + a[i + 3] * b[i + 3];
        i += 4;
    }
    while i < n {
        sum += a[i] * b[i];
        i += 1;
    }
    sum
}

/// Dot product dispatched to the assembly kernel when the CPU allows it.
#[inline]
pub fn dot_fast(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        if x86::has_fma() {
            // SAFETY: the FMA path reads exactly n elements from both slices.
            return unsafe { x86::dot_fma(a.as_ptr(), b.as_ptr(), a.len().min(b.len())) };
        }
    }
    dot(a, b)
}
