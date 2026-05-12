//! FTRL-Proximal logistic regression.
//!
//! Per-coordinate adaptive learning rate.  State: (z, n) vectors + hyperparams.
//! Update and predict both O(d).  p50 < 500 ns for d ≤ 64.

use super::OnlineLearner;

/// FTRL-Proximal hyperparameters.
#[derive(Clone, Copy, Debug)]
pub struct FtrlConfig {
    /// Per-coordinate learning rate.
    pub alpha: f32,
    /// Learning-rate smoothing (prevents large lr at step 0).
    pub beta: f32,
    /// L1 regularization strength.
    pub l1: f32,
    /// L2 regularization strength.
    pub l2: f32,
}

impl Default for FtrlConfig {
    fn default() -> Self {
        Self {
            alpha: 0.1,
            beta: 1.0,
            l1: 0.0,
            l2: 0.0,
        }
    }
}

/// FTRL-Proximal logistic regression learner.
#[derive(Clone, Debug)]
pub struct FtrlLearner {
    cfg: FtrlConfig,
    /// Per-feature z accumulator.
    z: Vec<f32>,
    /// Per-feature n (sum of squared gradients).
    n: Vec<f32>,
    dim: usize,
}

impl FtrlLearner {
    /// Create with given dimensionality and default hyperparams.
    pub fn new(dim: usize) -> Self {
        Self::with_config(dim, FtrlConfig::default())
    }

    pub fn with_config(dim: usize, cfg: FtrlConfig) -> Self {
        Self {
            cfg,
            z: vec![0.0; dim],
            n: vec![0.0; dim],
            dim,
        }
    }

    /// Compute weight vector w from z, n (not stored — derived on the fly).
    fn weights(&self) -> Vec<f32> {
        let mut w = vec![0.0f32; self.dim];
        for i in 0..self.dim {
            let zi = self.z[i];
            if zi.abs() <= self.cfg.l1 {
                w[i] = 0.0;
            } else {
                let sign = if zi > 0.0 { 1.0 } else { -1.0 };
                let lr_i = (self.cfg.beta + self.n[i].sqrt()) / self.cfg.alpha + self.cfg.l2;
                w[i] = -(zi - sign * self.cfg.l1) / lr_i;
            }
        }
        w
    }

    fn dot(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
    }

    fn sigmoid(x: f32) -> f32 {
        1.0 / (1.0 + (-x).exp())
    }
}

impl OnlineLearner for FtrlLearner {
    fn update(&mut self, features: &[f32], y: f32) -> f32 {
        let len = features.len().min(self.dim);
        let w = self.weights();
        let p = Self::sigmoid(Self::dot(&w[..len], &features[..len]));
        let grad = p - y;
        // Log-loss
        let loss = -(y * p.max(1e-7).ln() + (1.0 - y) * (1.0 - p).max(1e-7).ln());
        for i in 0..len {
            let g = grad * features[i];
            let sigma = (((self.n[i] + g * g).sqrt() - self.n[i].sqrt()) / self.cfg.alpha).max(0.0);
            self.z[i] += g - sigma * w[i];
            self.n[i] += g * g;
        }
        loss
    }

    fn predict(&self, features: &[f32]) -> f32 {
        let len = features.len().min(self.dim);
        let w = self.weights();
        Self::sigmoid(Self::dot(&w[..len], &features[..len]))
    }

    fn serialize(&self) -> Vec<u8> {
        // Layout: [version u8=1][dim u32 LE][alpha f32][beta f32][l1 f32][l2 f32][z…][n…]
        let mut buf = Vec::with_capacity(1 + 4 + 4 * 4 + self.dim * 8);
        buf.push(1u8); // version
        buf.extend_from_slice(&(self.dim as u32).to_le_bytes());
        buf.extend_from_slice(&self.cfg.alpha.to_le_bytes());
        buf.extend_from_slice(&self.cfg.beta.to_le_bytes());
        buf.extend_from_slice(&self.cfg.l1.to_le_bytes());
        buf.extend_from_slice(&self.cfg.l2.to_le_bytes());
        for &v in &self.z {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        for &v in &self.n {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf
    }

    fn deserialize_from(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || bytes[0] != 1 {
            return None;
        }
        let mut pos = 1usize;
        let read_u32 = |pos: &mut usize| -> Option<u32> {
            let b = bytes.get(*pos..*pos + 4)?;
            *pos += 4;
            Some(u32::from_le_bytes(b.try_into().ok()?))
        };
        let read_f32 = |pos: &mut usize| -> Option<f32> {
            let b = bytes.get(*pos..*pos + 4)?;
            *pos += 4;
            Some(f32::from_le_bytes(b.try_into().ok()?))
        };
        let dim = read_u32(&mut pos)? as usize;
        let alpha = read_f32(&mut pos)?;
        let beta = read_f32(&mut pos)?;
        let l1 = read_f32(&mut pos)?;
        let l2 = read_f32(&mut pos)?;
        let mut z = Vec::with_capacity(dim);
        let mut n = Vec::with_capacity(dim);
        for _ in 0..dim {
            z.push(read_f32(&mut pos)?);
        }
        for _ in 0..dim {
            n.push(read_f32(&mut pos)?);
        }
        Some(Self {
            cfg: FtrlConfig {
                alpha,
                beta,
                l1,
                l2,
            },
            z,
            n,
            dim,
        })
    }
}
