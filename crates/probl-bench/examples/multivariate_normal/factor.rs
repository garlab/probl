//! Numerical prototype only: no language API, cache, or engine resource budget.
//! Pivoted Cholesky on correlation scale, with a full reconstruction check.
use probl_engine::continuous::Rng;

pub const MAX_DIMENSION: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Shape,
    Dimension,
    NonFinite,
    NegativeVariance,
    ZeroVarianceCovariance,
    Asymmetric,
    NotPositiveSemidefinite,
    NonFiniteDraw,
}

#[derive(Debug)]
pub struct Factor {
    pub dimension: usize,
    pub rank: usize,
    pub residual: f64,
    pub tolerance: f64,
    // Original coordinate order, with columns in deterministic pivot order.
    pub lower: Vec<f64>,
    pub scales: Vec<f64>,
}

impl Factor {
    pub fn new(covariance: &[Vec<f64>]) -> Result<Self, Error> {
        let n = covariance.len();
        if n == 0 || covariance.iter().any(|row| row.len() != n) {
            return Err(Error::Shape);
        }
        if n > MAX_DIMENSION {
            return Err(Error::Dimension);
        }
        if covariance.iter().flatten().any(|x| !x.is_finite()) {
            return Err(Error::NonFinite);
        }
        if (0..n).any(|i| covariance[i][i] < 0.0) {
            return Err(Error::NegativeVariance);
        }
        let scales: Vec<_> = (0..n).map(|i| libm::sqrt(covariance[i][i])).collect();
        let tolerance = 64.0 * n as f64 * f64::EPSILON;
        let mut correlation = vec![0.0; n * n];
        for i in 0..n {
            correlation[i * n + i] = if scales[i] == 0.0 { 0.0 } else { 1.0 };
            for j in 0..i {
                let a = covariance[i][j];
                let b = covariance[j][i];
                if scales[i] == 0.0 || scales[j] == 0.0 {
                    if a != 0.0 || b != 0.0 {
                        return Err(Error::ZeroVarianceCovariance);
                    }
                    continue;
                }
                // Avoid overflow/underflow in sd_i * sd_j. For a valid
                // covariance, division by the larger SD cannot overflow.
                let large = scales[i].max(scales[j]);
                let small = scales[i].min(scales[j]);
                let a = a / large / small;
                let b = b / large / small;
                if !a.is_finite() || !b.is_finite() || a.abs().max(b.abs()) > 1.0 + tolerance {
                    return Err(Error::NotPositiveSemidefinite);
                }
                if (a - b).abs() > tolerance {
                    return Err(Error::Asymmetric);
                }
                let midpoint = a + (b - a) * 0.5;
                correlation[i * n + j] = midpoint;
                correlation[j * n + i] = midpoint;
            }
        }

        let mut lower = vec![0.0; n * n];
        let mut selected = vec![false; n];
        let mut rank = 0;
        for k in 0..n {
            let mut pivot = None;
            let mut largest = f64::NEG_INFINITY;
            for i in 0..n {
                if !selected[i] {
                    let diagonal = correlation[i * n + i] - dot(&lower, n, i, i, k);
                    // Strict comparison breaks ties by original coordinate.
                    if diagonal > largest {
                        largest = diagonal;
                        pivot = Some(i);
                    }
                }
            }
            if largest <= tolerance {
                break;
            }
            let p = pivot.expect("an unselected coordinate");
            let root = libm::sqrt(largest);
            lower[p * n + k] = root;
            selected[p] = true;
            for i in 0..n {
                if !selected[i] {
                    lower[i * n + k] = (correlation[i * n + p] - dot(&lower, n, i, p, k)) / root;
                }
            }
            rank += 1;
        }
        // A small remaining diagonal alone does NOT prove PSD. Check every
        // entry, including off-diagonal residuals in a rank-deficient tail.
        let mut residual = 0.0_f64;
        for i in 0..n {
            for j in 0..=i {
                let error = (correlation[i * n + j] - dot(&lower, n, i, j, rank)).abs();
                if !error.is_finite() || error > tolerance {
                    return Err(Error::NotPositiveSemidefinite);
                }
                residual = residual.max(error);
            }
        }
        Ok(Self {
            dimension: n,
            rank,
            residual,
            tolerance,
            lower,
            scales,
        })
    }

    pub fn sample(&self, means: &[f64], rng: &mut Rng) -> Result<Vec<f64>, Error> {
        if means.len() != self.dimension {
            return Err(Error::Shape);
        }
        if means.iter().any(|x| !x.is_finite()) {
            return Err(Error::NonFinite);
        }
        let z: Vec<_> = (0..self.rank).map(|_| rng.normal()).collect();
        let mut result = Vec::with_capacity(self.dimension);
        for (i, mean) in means.iter().enumerate() {
            let mut deviation = 0.0;
            for (j, normal) in z.iter().enumerate() {
                deviation += self.lower[i * self.dimension + j] * normal;
            }
            let value = mean + self.scales[i] * deviation;
            if !value.is_finite() {
                return Err(Error::NonFiniteDraw);
            }
            result.push(value);
        }
        Ok(result)
    }

    pub fn normalized_covariance(&self, i: usize, j: usize) -> f64 {
        dot(&self.lower, self.dimension, i, j, self.rank)
    }
}

fn dot(lower: &[f64], n: usize, i: usize, j: usize, rank: usize) -> f64 {
    let mut sum = 0.0;
    for k in 0..rank {
        sum += lower[i * n + k] * lower[j * n + k];
    }
    sum
}
