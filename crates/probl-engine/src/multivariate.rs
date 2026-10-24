//! Bounded multivariate-normal recipes and constructor cache.
//! Numerical rules match docs/multivariate-normal-prototype.md. Growing buffers
//! reserve live bytes and use fallible allocation; fixed Arc headers use the
//! standard allocator after reserving their accounted size.
mod memory;
#[cfg(test)]
mod tests;

use crate::continuous::Rng;
use crate::dist::Budget;
use crate::error::{Fault, OpError, OpResult};
use crate::value::Value;
pub use memory::Resources;
use memory::{Buffer, Reservation};
use std::mem::size_of;
use std::sync::Arc;

pub const MAX_DIMENSION: usize = 64;
/// Function identity and source offset, supplied by the interpreter caller.
pub type Site = (usize, usize);

fn domain(message: impl Into<String>) -> OpError {
    OpError::fault(Fault::DomainError, message)
}

fn same_execution(resources: &Resources, budget: &Budget) -> OpResult<()> {
    if resources.same(&budget.multivariate) {
        Ok(())
    } else {
        Err(OpError::internal("multivariate storage belongs to another execution"))
    }
}

fn validate(means: &[f64], covariance: &[impl AsRef<[f64]>], budget: &mut Budget) -> OpResult<usize> {
    budget.work(1)?;
    let n = covariance.len();
    if n == 0 {
        return Err(domain("multivariate_normal needs a nonempty covariance matrix"));
    }
    budget.multivariate.dimension(n)?;
    budget.collection(n as u128)?;
    budget.work(n as u64)?;
    if means.len() != n {
        return Err(domain("multivariate_normal means and covariance dimensions differ"));
    }
    for (i, &value) in means.iter().enumerate() {
        if !value.is_finite() {
            return Err(domain(format!("multivariate_normal mean at index {i} must be finite")));
        }
    }
    for (i, row) in covariance.iter().enumerate() {
        let row = row.as_ref();
        budget.work(n as u64)?;
        if row.len() != n {
            return Err(domain(format!(
                "multivariate_normal covariance row {i} must have {n} entries"
            )));
        }
        for (j, &value) in row.iter().enumerate() {
            if !value.is_finite() {
                return Err(domain(format!(
                    "multivariate_normal covariance at [{i}, {j}] must be finite"
                )));
            }
        }
    }
    Ok(n)
}

#[derive(Debug)]
pub struct Factor {
    dimension: usize,
    rank: usize,
    residual: f64,
    // Covariance, factor coefficients and scales share one owned allocation.
    storage: Buffer<f64>,
    _memory: Reservation,
}

impl Factor {
    pub fn dimension(&self) -> usize {
        self.dimension
    }
    pub fn rank(&self) -> usize {
        self.rank
    }
    pub fn residual(&self) -> f64 {
        self.residual
    }
    pub fn coefficients(&self) -> &[f64] {
        let square = self.dimension * self.dimension;
        &self.storage[square..2 * square]
    }
    pub fn covariance(&self) -> &[f64] {
        &self.storage[..self.dimension * self.dimension]
    }
    fn scales(&self) -> &[f64] {
        &self.storage[2 * self.dimension * self.dimension..]
    }

    // Called only after bounded shape and finite-entry validation.
    fn build(input: &[impl AsRef<[f64]>], budget: &mut Budget) -> OpResult<Arc<Self>> {
        let n = input.len();
        budget.work((3 * n * n + 2 * n) as u64)?;
        let resources = budget.multivariate.clone();
        let memory = resources.reserve(size_of::<Self>() + 2 * size_of::<usize>())?;
        let mut storage = Buffer::filled(2 * n * n + n, 0.0, &resources)?;
        let (covariance, rest) = storage.split_at_mut(n * n);
        let (lower, scales) = rest.split_at_mut(n * n);
        let mut correlation = Buffer::filled(n * n, 0.0, &resources)?;
        let tolerance = 64.0 * n as f64 * f64::EPSILON;
        for i in 0..n {
            budget.work((n + 1) as u64)?;
            let row = input[i].as_ref();
            if row[i] < 0.0 {
                return Err(domain(format!(
                    "multivariate_normal variance at [{i}, {i}] must be nonnegative"
                )));
            }
            scales[i] = libm::sqrt(row[i]);
            for j in 0..n {
                covariance[i * n + j] = if row[j] == 0.0 { 0.0 } else { row[j] };
            }
        }
        for i in 0..n {
            budget.work((1 + 8 * i) as u64)?;
            correlation[i * n + i] = if scales[i] == 0.0 { 0.0 } else { 1.0 };
            for j in 0..i {
                let a = covariance[i * n + j];
                let b = covariance[j * n + i];
                if scales[i] == 0.0 || scales[j] == 0.0 {
                    if a != 0.0 || b != 0.0 {
                        return Err(domain(format!(
                            "multivariate_normal zero variance requires zero covariance at [{i}, {j}] and [{j}, {i}]"
                        )));
                    }
                    continue;
                }
                let large = scales[i].max(scales[j]);
                let small = scales[i].min(scales[j]);
                let a = a / large / small;
                let b = b / large / small;
                if !a.is_finite() || !b.is_finite() || a.abs().max(b.abs()) > 1.0 + tolerance {
                    return Err(domain(format!(
                        "multivariate_normal covariance at [{i}, {j}] is not positive semidefinite within tolerance"
                    )));
                }
                if (a - b).abs() > tolerance {
                    return Err(domain(format!(
                        "multivariate_normal covariance at [{i}, {j}] and [{j}, {i}] must be symmetric within tolerance"
                    )));
                }
                let midpoint = a + (b - a) * 0.5;
                correlation[i * n + j] = midpoint;
                correlation[j * n + i] = midpoint;
            }
        }
        // Fixed, bounded stack scratch avoids a tiny allocation/reservation.
        let mut selected = [false; MAX_DIMENSION];
        let mut rank = 0;
        for k in 0..n {
            let mut pivot = None;
            let mut largest = f64::NEG_INFINITY;
            for i in 0..n {
                budget.work((k + 1) as u64)?;
                if !selected[i] {
                    let diagonal = correlation[i * n + i] - dot(lower, n, i, i, k);
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
                budget.work((k + 2) as u64)?;
                if !selected[i] {
                    lower[i * n + k] = (correlation[i * n + p] - dot(lower, n, i, p, k)) / root;
                }
            }
            rank += 1;
        }
        let mut residual = 0.0_f64;
        for i in 0..n {
            budget.work(((i + 1) * (rank + 1)) as u64)?;
            for j in 0..=i {
                let error = (correlation[i * n + j] - dot(lower, n, i, j, rank)).abs();
                if !error.is_finite() || error > tolerance {
                    return Err(domain(format!(
                        "multivariate_normal covariance is not positive semidefinite within tolerance at [{i}, {j}]"
                    )));
                }
                residual = residual.max(error);
            }
        }
        Ok(Arc::new(Self {
            dimension: n,
            rank,
            residual,
            storage,
            _memory: memory,
        }))
    }
}

#[derive(Debug)]
pub struct Normal {
    means: Buffer<f64>,
    factor: Arc<Factor>,
    _memory: Reservation,
}

impl Normal {
    pub fn new(means: &[f64], covariance: &[Vec<f64>], budget: &mut Budget) -> OpResult<Arc<Self>> {
        Cache::new(0, budget).normal((0, 0), means, covariance, budget)
    }

    pub fn means(&self) -> &[f64] {
        &self.means
    }
    pub fn factor(&self) -> &Arc<Factor> {
        &self.factor
    }

    /// Canonical finite input identity, independent of factor/cache/RNG state.
    pub(crate) fn identity(&self) -> impl Iterator<Item = u64> + '_ {
        std::iter::once(self.means.len() as u64).chain(
            self.means
                .iter()
                .chain(self.factor.covariance().iter())
                .map(|x| x.to_bits()),
        )
    }

    pub(crate) fn sample_value(&self, rng: &mut Rng, budget: &mut Budget) -> OpResult<Value> {
        // Build the language list directly, without an uncharged numeric temporary.
        self.sample_as(rng, budget, Value::Float).map(Value::list)
    }

    fn with_factor(means: &[f64], factor: Arc<Factor>, budget: &mut Budget) -> OpResult<Arc<Self>> {
        budget.work(means.len() as u64)?;
        let memory = budget
            .multivariate
            .reserve(size_of::<Self>() + 2 * size_of::<usize>())?;
        let mut values = Buffer::filled(means.len(), 0.0, &budget.multivariate)?;
        for (out, &value) in values.iter_mut().zip(means) {
            *out = if value == 0.0 { 0.0 } else { value };
        }
        Ok(Arc::new(Self {
            means: values,
            factor,
            _memory: memory,
        }))
    }

    pub fn sample(&self, rng: &mut Rng, budget: &mut Budget) -> OpResult<Vec<f64>> {
        self.sample_as(rng, budget, |x| x)
    }

    fn sample_as<T>(&self, rng: &mut Rng, budget: &mut Budget, convert: impl Fn(f64) -> T) -> OpResult<Vec<T>> {
        same_execution(&self._memory.resources, budget)?;
        let f = &self.factor;
        let n = f.dimension;
        budget.collection(n as u128)?;
        // Preflight work and allocations before advancing the model stream.
        budget.work((1 + f.rank + n * (f.rank + 2)) as u64)?;
        let mut z = Buffer::filled(f.rank, 0.0, &budget.multivariate)?;
        let mut result = Vec::new();
        result
            .try_reserve_exact(n)
            .map_err(|_| OpError::limit("cannot allocate multivariate sample list"))?;
        for value in z.iter_mut() {
            budget.work(0)?;
            *value = rng.normal();
        }
        for i in 0..n {
            budget.work(0)?;
            let mut deviation = 0.0;
            for (j, normal) in z.iter().enumerate() {
                deviation += f.coefficients()[i * n + j] * normal;
            }
            let value = self.means[i] + f.scales()[i] * deviation;
            if !value.is_finite() {
                return Err(OpError::fault(
                    Fault::NumericOverflow,
                    "multivariate_normal produced a nonfinite sample",
                ));
            }
            result.push(convert(value));
        }
        Ok(result)
    }
}

#[derive(Debug)]
struct Entry {
    site: Site,
    factor: Arc<Factor>,
}

/// One last covariance per constructor site. The interpreter must clear this
/// at batch boundaries; exceeding the entry cap clears it as existing call caches do.
#[derive(Debug)]
pub struct Cache {
    entries: Buffer<Entry>,
    resources: Resources,
    max_entries: usize,
}

impl Cache {
    pub fn new(max_entries: usize, budget: &Budget) -> Self {
        Self {
            entries: Buffer::empty(&budget.multivariate),
            resources: budget.multivariate.clone(),
            max_entries,
        }
    }

    pub fn clear(&mut self) {
        self.entries = Buffer::empty(&self.resources);
    }

    pub fn normal(
        &mut self,
        site: Site,
        means: &[f64],
        covariance: &[impl AsRef<[f64]>],
        budget: &mut Budget,
    ) -> OpResult<Arc<Normal>> {
        same_execution(&self.resources, budget)?;
        let n = validate(means, covariance, budget)?;
        budget.work(u64::from(self.entries.len().checked_ilog2().unwrap_or(0)) + 1)?;
        let index = self.entries.binary_search_by_key(&site, |entry| entry.site);
        let mut hit = None;
        if let Ok(i) = index {
            let factor = &self.entries[i].factor;
            if factor.dimension == n {
                budget.work((n * n) as u64)?;
                if factor
                    .covariance()
                    .iter()
                    .copied()
                    .eq(covariance.iter().flat_map(|row| row.as_ref().iter()).copied())
                {
                    hit = Some(factor.clone());
                }
            }
        }
        let cached = hit.is_some();
        let factor = match hit {
            Some(factor) => factor,
            None => Factor::build(covariance, budget)?,
        };
        let normal = Normal::with_factor(means, factor.clone(), budget)?;
        if !cached && self.max_entries > 0 {
            match index {
                Ok(i) => self.entries[i].factor = factor,
                Err(mut i) => {
                    budget.work((self.entries.len() + 1) as u64)?;
                    if self.entries.len() == self.max_entries {
                        self.clear();
                        i = 0;
                    }
                    let needed = self.entries.len() + 1;
                    if needed > self.entries.capacity() {
                        let capacity = needed
                            .max(self.entries.capacity().saturating_mul(2))
                            .min(self.max_entries);
                        self.entries.grow(capacity)?;
                    }
                    self.entries.insert(i, Entry { site, factor });
                }
            }
        }
        Ok(normal)
    }

    /// Convert settled language lists with charged, fallible temporary buffers.
    pub(crate) fn normal_values(&mut self, site: Site, args: &[Value], budget: &mut Budget) -> OpResult<Value> {
        fn list<'a>(value: &'a Value, name: &str) -> OpResult<&'a [Value]> {
            match value {
                Value::List(values) => Ok(values),
                _ => Err(OpError::new(format!(
                    "multivariate_normal {name} must be a settled list"
                ))),
            }
        }
        fn number(value: &Value, i: usize, j: Option<usize>, budget: &mut Budget) -> OpResult<f64> {
            let at = || match j {
                Some(j) => format!("covariance at [{i}, {j}]"),
                None => format!("mean at index {i}"),
            };
            let x = match value {
                Value::Int(n) => {
                    budget.integer_work(n, &probl_number::Integer::ONE, false)?;
                    n.to_f64().ok_or_else(|| {
                        OpError::fault(
                            Fault::NumericOverflow,
                            format!("multivariate_normal {} does not fit in a finite float", at()),
                        )
                    })?
                }
                Value::Float(x) | Value::Prob(x) => *x,
                _ => {
                    return Err(OpError::new(format!(
                        "multivariate_normal {} must be a settled real number",
                        at()
                    )));
                }
            };
            if !x.is_finite() {
                return Err(domain(format!("multivariate_normal {} must be finite", at())));
            }
            Ok(x)
        }
        budget.work(1)?;
        let means = list(&args[0], "means")?;
        let rows = list(&args[1], "covariance")?;
        let n = rows.len();
        budget.multivariate.dimension(n)?;
        budget.collection(n as u128)?;
        if n == 0 || means.len() != n {
            return Err(domain(
                "multivariate_normal needs matching nonempty means and covariance dimensions",
            ));
        }
        // Check every row's shape before allocating or converting numeric entries.
        for (i, row) in rows.iter().enumerate() {
            budget.work(1)?;
            let Value::List(row) = row else {
                return Err(OpError::new(format!(
                    "multivariate_normal covariance row {i} must be a settled list"
                )));
            };
            if row.len() != n {
                return Err(domain(format!(
                    "multivariate_normal covariance row {i} must have {n} entries"
                )));
            }
        }
        let resources = budget.multivariate.clone();
        budget.work(n as u64)?;
        // One contiguous conversion allocation instead of a charged allocation
        // and reference-count update for every row. Dimensions are already bounded.
        let mut converted = Buffer::filled(n + n * n, 0.0, &resources)?;
        for (i, value) in means.iter().enumerate() {
            converted[i] = number(value, i, None, budget)?;
        }
        for (i, row) in rows.iter().enumerate() {
            budget.work(n as u64)?;
            let Value::List(row) = row else {
                unreachable!("validated shape")
            };
            for (j, value) in row.iter().enumerate() {
                converted[n + i * n + j] = number(value, i, Some(j), budget)?;
            }
        }
        // Only row views live on the stack; all converted numeric payload is charged.
        let mut covariance: [&[f64]; MAX_DIMENSION] = [&[]; MAX_DIMENSION];
        for (row, values) in covariance.iter_mut().zip(converted[n..].chunks_exact(n)) {
            *row = values;
        }
        self.normal(site, &converted[..n], &covariance[..n], budget)
            .map(Value::MultivariateNormal)
    }
}

/// Unsupported operations must not interpret a recipe as a categorical atom.
pub(crate) fn unsupported(operation: &str) -> OpError {
    OpError::unsupported(format!("{operation} is not supported for multivariate_normal"))
        .help("draw a vector in sample mode, then work with its components")
}

/// A recipe or finite mixture containing one (not a list storing recipes).
pub(crate) fn is_law(value: &Value) -> bool {
    match value {
        Value::MultivariateNormal(_) => true,
        Value::Dist(d) | Value::Joint(d) => d.outcomes.iter().any(|(v, _)| is_law(v)),
        _ => false,
    }
}

fn dot(lower: &[f64], n: usize, i: usize, j: usize, rank: usize) -> f64 {
    let mut sum = 0.0;
    for k in 0..rank {
        sum += lower[i * n + k] * lower[j * n + k];
    }
    sum
}
