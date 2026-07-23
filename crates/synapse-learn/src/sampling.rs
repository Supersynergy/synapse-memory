use rand::{Rng, RngExt};

/// Draw from Beta(alpha, beta) through two independent Gamma samples.
///
/// The learning priors are positive integer counts, but the implementation also
/// handles positive fractional shapes for callers inside this crate.
pub(crate) fn beta<R: Rng + ?Sized>(rng: &mut R, alpha: f64, beta: f64) -> Option<f64> {
    let x = gamma(rng, alpha)?;
    let y = gamma(rng, beta)?;
    let total = x + y;
    (total.is_finite() && total > 0.0).then_some(x / total)
}

fn gamma<R: Rng + ?Sized>(rng: &mut R, shape: f64) -> Option<f64> {
    if !shape.is_finite() || shape <= 0.0 {
        return None;
    }
    if shape < 1.0 {
        let u = rng.random::<f64>().max(f64::MIN_POSITIVE);
        return gamma(rng, shape + 1.0).map(|sample| sample * u.powf(1.0 / shape));
    }

    // Marsaglia-Tsang rejection sampler for Gamma(shape, scale=1).
    let d = shape - 1.0 / 3.0;
    let c = (9.0 * d).sqrt().recip();
    for _ in 0..128 {
        let x = standard_normal(rng);
        let base = 1.0 + c * x;
        if base <= 0.0 {
            continue;
        }
        let v = base * base * base;
        let u = rng.random::<f64>().max(f64::MIN_POSITIVE);
        if u < 1.0 - 0.0331 * x.powi(4) || u.ln() < 0.5 * x * x + d * (1.0 - v + v.ln()) {
            return Some(d * v);
        }
    }
    None
}

fn standard_normal<R: Rng + ?Sized>(rng: &mut R) -> f64 {
    let u1 = rng.random::<f64>().max(f64::MIN_POSITIVE);
    let u2 = rng.random::<f64>();
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

    #[test]
    fn beta_samples_have_expected_range_and_mean() {
        let mut rng = StdRng::seed_from_u64(7);
        let samples: Vec<f64> = (0..10_000)
            .map(|_| beta(&mut rng, 2.0, 3.0).expect("valid beta sample"))
            .collect();
        assert!(samples.iter().all(|sample| (0.0..=1.0).contains(sample)));
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        assert!((mean - 0.4).abs() < 0.02, "unexpected mean: {mean}");
    }

    #[test]
    fn beta_rejects_invalid_shapes() {
        let mut rng = StdRng::seed_from_u64(9);
        assert!(beta(&mut rng, 0.0, 1.0).is_none());
        assert!(beta(&mut rng, 1.0, f64::NAN).is_none());
    }
}
