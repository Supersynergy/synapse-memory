//! AMX/Accelerate correlation-matrix kernel — Apple Silicon fast path.
//!
//! On macOS aarch64: `cblas_sgemm` via Accelerate framework (AMX backed, ~117× vs scalar).
//! Elsewhere: per-pair NEON `correlation_f32` fallback.

/// Full n×n Pearson correlation matrix for a row-major f32 matrix.
///
/// `mat`: `rows` observations × `cols` variables, row-major.
/// Returns `cols × cols` correlation matrix, row-major.
pub fn correlation_matrix_amx(mat: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    assert_eq!(mat.len(), rows * cols, "mat.len() must equal rows*cols");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return amx_impl(mat, rows, cols);
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    return neon_fallback(mat, rows, cols);
}

// ── macOS aarch64: Accelerate cblas_sgemm ─────────────────────────────────────

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[link(name = "Accelerate", kind = "framework")]
unsafe extern "C" {
    fn cblas_sgemm(
        order: u32,
        transa: u32,
        transb: u32,
        m: i32,
        n: i32,
        k: i32,
        alpha: f32,
        a: *const f32,
        lda: i32,
        b: *const f32,
        ldb: i32,
        beta: f32,
        c: *mut f32,
        ldc: i32,
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const CBLAS_ROW_MAJOR: u32 = 101;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const CBLAS_TRANS: u32 = 112;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const CBLAS_NO_TRANS: u32 = 111;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn amx_impl(mat: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    let n = rows;
    let p = cols;

    // Normalize each column: subtract mean, divide by std
    let mut normed = mat.to_vec();
    for c in 0..p {
        let mean = (0..n).map(|r| normed[r * p + c]).sum::<f32>() / n as f32;
        let var = (0..n)
            .map(|r| {
                let d = normed[r * p + c] - mean;
                d * d
            })
            .sum::<f32>()
            / n as f32;
        let inv = if var < 1e-24 { 0.0 } else { 1.0 / var.sqrt() };
        for r in 0..n {
            normed[r * p + c] = (normed[r * p + c] - mean) * inv;
        }
    }

    // C = (1/n) * Nᵀ · N  →  p×p correlation matrix
    let mut out = vec![0.0f32; p * p];
    unsafe {
        cblas_sgemm(
            CBLAS_ROW_MAJOR,
            CBLAS_TRANS, // A = Nᵀ
            CBLAS_NO_TRANS,
            p as i32, // M
            p as i32, // N
            n as i32, // K
            1.0 / n as f32,
            normed.as_ptr(),
            p as i32, // lda
            normed.as_ptr(),
            p as i32, // ldb
            0.0,
            out.as_mut_ptr(),
            p as i32, // ldc
        );
    }

    for i in 0..p {
        out[i * p + i] = 1.0;
    }
    out
}

// ── portable fallback (wide f32x8 NEON path) ──────────────────────────────────

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn neon_fallback(mat: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    use super::neon::correlation_f32;
    let n = rows;
    let p = cols;
    let mut out = vec![0.0f32; p * p];
    for i in 0..p {
        for j in i..p {
            let a: Vec<f32> = (0..n).map(|r| mat[r * p + i]).collect();
            let b: Vec<f32> = (0..n).map(|r| mat[r * p + j]).collect();
            let r = correlation_f32(&a, &b);
            out[i * p + j] = r;
            out[j * p + i] = r;
        }
        out[i * p + i] = 1.0;
    }
    out
}

// ── CorrMatrix result type ─────────────────────────────────────────────────────

/// n×n Pearson correlation matrix (row-major).
pub struct CorrMatrix {
    pub data: Vec<f32>,
    pub n: usize,
}

impl CorrMatrix {
    #[inline]
    pub fn get(&self, i: usize, j: usize) -> f32 {
        self.data[i * self.n + j]
    }
}
