//! Budgeted version of the lazy compactor evaluated in docs/quantile-sketches.md.
//! Every growing buffer reserves both allocations before moving any data. The
//! query view is frozen once, shared by results, and retains its own reservation.
use super::memory::{Reservation, SummaryBudget};
use crate::error::{OpError, OpResult};
use std::mem::size_of;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// Retention/accuracy metadata, separate from Monte Carlo sampling uncertainty.
/// Capacity controls space and measured accuracy, not a guaranteed rank error.
/// More fields may be added, so it's only constructed by Probl.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct QuantileApproximation {
    pub capacity: usize,
    pub samples: u64,
    pub zeros: u64,
}

impl QuantileApproximation {
    /// Algorithm and query convention version; all quantiles use inverse CDF.
    pub fn method(self) -> &'static str {
        "compactor_v1"
    }
}

pub(super) fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn allocation() -> OpError {
    OpError::limit("cannot allocate quantile report storage within max_report_summary_bytes")
}

#[derive(Clone, Debug)]
struct Buffer<T> {
    values: Vec<T>,
    // Freed after the vector. Host snapshot clones detach their reservation.
    memory: Reservation,
}

impl<T> Buffer<T> {
    fn new(budget: &SummaryBudget) -> OpResult<Self> {
        Ok(Self {
            values: Vec::new(),
            memory: budget.reserve(0)?,
        })
    }

    fn resize_storage(&mut self, capacity: usize, budget: &SummaryBudget) -> OpResult<()> {
        let bytes = capacity.checked_mul(size_of::<T>()).ok_or_else(allocation)?;
        let memory = budget.reserve(bytes)?;
        let mut values = Vec::new();
        values.try_reserve_exact(capacity).map_err(|_| allocation())?;
        // Vec records the requested element capacity with the system allocator;
        // allocator metadata/rounding, like the other collector charges, is out
        // of scope. Do not use amortized reserve here.
        values.append(&mut self.values);
        let old = std::mem::replace(&mut self.values, values);
        drop(old);
        self.memory = memory;
        Ok(())
    }

    fn ensure(&mut self, needed: usize, budget: &SummaryBudget) -> OpResult<()> {
        if needed > self.values.capacity() {
            let capacity = needed.max(self.values.capacity().saturating_mul(2)).max(4);
            self.resize_storage(capacity, budget)?;
        }
        Ok(())
    }

    fn push(&mut self, value: T, budget: &SummaryBudget) -> OpResult<()> {
        self.ensure(self.len().checked_add(1).ok_or_else(allocation)?, budget)?;
        self.values.push(value);
        Ok(())
    }
}
impl<T> Deref for Buffer<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.values
    }
}
impl<T> DerefMut for Buffer<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.values
    }
}

#[derive(Clone, Debug)]
pub(super) struct Sketch {
    state: State,
    // Accounts for the enclosing Box, after its contents are dropped.
    _memory: Reservation,
}

#[derive(Clone, Debug)]
enum State {
    Collect(Collector),
    Frozen(Arc<Quantiles>),
}

impl Sketch {
    pub fn new(capacity: usize, seed: u64, budget: &SummaryBudget) -> OpResult<Box<Self>> {
        if !(128..=4096).contains(&capacity) {
            return Err(OpError::new("quantile capacity must be within 128..=4096"));
        }
        let memory = budget.reserve(size_of::<Self>())?;
        let mut levels = Buffer::new(budget)?;
        levels.push(Buffer::new(budget)?, budget)?;
        Ok(Box::new(Self {
            state: State::Collect(Collector {
                capacity,
                levels,
                size: 0,
                limit: capacity,
                random: seed,
                count: 0,
                negative: 0,
                zeros: 0,
                min: f64::INFINITY,
                max: f64::NEG_INFINITY,
                budget: budget.clone(),
            }),
            _memory: memory,
        }))
    }

    pub fn add(&mut self, value: f64) -> OpResult<()> {
        let State::Collect(c) = &mut self.state else {
            return Err(OpError::internal("adding to a finished sketch"));
        };
        c.budget.check()?;
        if !value.is_finite() {
            return Err(OpError::unsupported("quantile collection requires finite numbers"));
        }
        let value = if value == 0.0 { 0.0 } else { value };
        c.count = c.count.checked_add(1).ok_or_else(allocation)?;
        c.negative += u64::from(value < 0.0);
        c.zeros += u64::from(value == 0.0);
        c.min = c.min.min(value);
        c.max = c.max.max(value);
        if value != 0.0 {
            c.levels[0].push(value, &c.budget)?;
            c.size += 1;
            c.compress()?;
        }
        Ok(())
    }

    pub fn merge(&mut self, other: Self) -> OpResult<()> {
        let (State::Collect(a), State::Collect(mut b)) = (&mut self.state, other.state) else {
            return Err(OpError::internal("merging finished quantile sketches"));
        };
        a.budget.check()?;
        if a.capacity != b.capacity {
            return Err(OpError::internal("merging different sketch capacities"));
        }
        a.count = a.count.checked_add(b.count).ok_or_else(allocation)?;
        a.negative += b.negative;
        a.zeros += b.zeros;
        a.min = a.min.min(b.min);
        a.max = a.max.max(b.max);
        while a.levels.len() < b.levels.len() {
            a.grow()?;
        }
        a.size += b.size;
        // Keep the source's reservation until its allocation is freed. Every
        // destination growth also retains the old destination's reservation.
        for (i, values) in b.levels.values.drain(..).enumerate() {
            a.budget.check()?;
            let needed = a.levels[i].len().checked_add(values.len()).ok_or_else(allocation)?;
            a.levels[i].ensure(needed, &a.budget)?;
            a.levels[i].values.extend_from_slice(&values);
        }
        a.compress()?;
        a.trim()
    }

    pub fn finish(&mut self) -> OpResult<()> {
        if let State::Collect(c) = &self.state {
            let view = c.snapshot()?;
            self.state = State::Frozen(view);
        }
        Ok(())
    }

    pub fn view(&self) -> Option<&Arc<Quantiles>> {
        match &self.state {
            State::Frozen(view) => Some(view),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct Collector {
    capacity: usize,
    levels: Buffer<Buffer<f64>>,
    size: usize,
    limit: usize,
    random: u64,
    count: u64,
    negative: u64,
    zeros: u64,
    min: f64,
    max: f64,
    budget: SummaryBudget,
}

impl Collector {
    fn level_capacity(&self, level: usize) -> usize {
        let mut capacity = self.capacity;
        for _ in level + 1..self.levels.len() {
            capacity = (2 * capacity).div_ceil(3).max(8);
        }
        capacity
    }
    fn grow(&mut self) -> OpResult<()> {
        if self.levels.len() >= 64 {
            return Err(allocation());
        }
        self.levels.push(Buffer::new(&self.budget)?, &self.budget)?;
        self.limit = (0..self.levels.len()).map(|i| self.level_capacity(i)).sum();
        Ok(())
    }
    fn compress(&mut self) -> OpResult<()> {
        let mut compacted = false;
        while self.size >= self.limit {
            self.budget.check()?;
            compacted = true;
            let level = (0..self.levels.len())
                .find(|&i| self.levels[i].len() >= self.level_capacity(i))
                .ok_or_else(|| OpError::internal("no full quantile compactor"))?;
            if level + 1 == self.levels.len() {
                self.grow()?;
            }
            let (lower, upper) = self.levels.split_at_mut(level + 1);
            let values = &mut lower[level].values;
            // At most O(capacity + 64) items; cancellation is checked between
            // levels and before/after this allocation-free bounded sort.
            values.sort_unstable_by(f64::total_cmp);
            self.budget.check()?;
            let odd = (values.len() % 2 == 1).then(|| values.pop().unwrap());
            self.random = self.random.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let parity = (mix(self.random) & 1) as usize;
            upper[0].ensure(upper[0].len() + values.len() / 2, &self.budget)?;
            upper[0].values.extend(values.iter().skip(parity).step_by(2).copied());
            self.size -= values.len() / 2;
            values.clear();
            values.extend(odd);
        }
        if compacted {
            self.trim()?;
        }
        Ok(())
    }
    fn trim(&mut self) -> OpResult<()> {
        for level in 0..self.levels.len() {
            let wanted = self.level_capacity(level).max(self.levels[level].len());
            if self.levels[level].values.capacity() > 2 * wanted {
                self.levels[level].resize_storage(wanted, &self.budget)?;
            }
        }
        Ok(())
    }
    fn snapshot(&self) -> OpResult<Arc<Quantiles>> {
        let memory = self.budget.reserve(size_of::<Quantiles>() + 2 * size_of::<usize>())?;
        let mut points = Buffer::new(&self.budget)?;
        points.ensure(self.size, &self.budget)?;
        for (level, values) in self.levels.iter().enumerate() {
            self.budget.check()?;
            points.values.extend(values.iter().map(|&v| (v, 1_u64 << level)));
        }
        points.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        self.budget.check()?;
        let split = points.partition_point(|&(v, _)| v < 0.0);
        let negative_weight = points[..split].iter().map(|p| p.1).sum();
        let positive_weight = points[split..].iter().map(|p| p.1).sum();
        Ok(Arc::new(Quantiles {
            points,
            split,
            negative_weight,
            positive_weight,
            negative: self.negative,
            metadata: QuantileApproximation {
                capacity: self.capacity,
                samples: self.count,
                zeros: self.zeros,
            },
            min: self.min,
            max: self.max,
            _memory: memory,
        }))
    }
}

#[derive(Debug)]
pub(super) struct Quantiles {
    points: Buffer<(f64, u64)>,
    split: usize,
    negative_weight: u64,
    positive_weight: u64,
    negative: u64,
    pub metadata: QuantileApproximation,
    min: f64,
    max: f64,
    _memory: Reservation,
}

impl Quantiles {
    /// Lower inverse CDF, with exact zero/sign masses and endpoints. Queries
    /// allocate nothing; result ownership keeps the bounded view alive.
    pub fn quantile(&self, q: f64) -> Option<f64> {
        let QuantileApproximation {
            samples: count, zeros, ..
        } = self.metadata;
        if count == 0 || !q.is_finite() || !(0.0..=1.0).contains(&q) {
            return None;
        }
        if q == 0.0 {
            return Some(self.min);
        }
        if q == 1.0 {
            return Some(self.max);
        }
        let target = (q * count as f64).ceil().max(1.0);
        let (points, weight, rank, count, fallback) = if target <= self.negative as f64 {
            (
                &self.points[..self.split],
                self.negative_weight,
                target,
                self.negative,
                self.min,
            )
        } else if target <= (self.negative + zeros) as f64 {
            return Some(0.0);
        } else {
            (
                &self.points[self.split..],
                self.positive_weight,
                target - (self.negative + zeros) as f64,
                count - self.negative - zeros,
                self.max,
            )
        };
        let target = rank * (weight as f64 / count as f64);
        let mut cumulative = 0_u64;
        for &(value, weight) in points {
            cumulative += weight;
            if cumulative as f64 >= target {
                return Some(value);
            }
        }
        Some(points.last().map_or(fallback, |p| p.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn fill(sketch: &mut Sketch, first: u64, count: u64) -> OpResult<()> {
        for i in first..first + count {
            let x = if i % 5 == 0 {
                0.0
            } else {
                (mix(i) >> 12) as f64 - (1_u64 << 51) as f64
            };
            sketch.add(x)?;
        }
        Ok(())
    }

    #[test]
    fn merge_conserves_mass_and_finished_views_share_their_reservation() {
        let budget = SummaryBudget::new(1 << 20, None);
        let mut all = Sketch::new(128, 42, &budget).unwrap();
        for i in 0..21 {
            let mut batch = Sketch::new(128, mix(i), &budget).unwrap();
            fill(&mut batch, i * 999, 999).unwrap();
            all.merge(*batch).unwrap();
            let State::Collect(c) = &all.state else { panic!() };
            let weight: u64 = c.levels.iter().enumerate().map(|(l, v)| (v.len() as u64) << l).sum();
            assert_eq!(weight + c.zeros, c.count);
            assert!(c.size < 3 * c.capacity + 9 * 64);
            let bytes = size_of::<Sketch>()
                + c.levels.values.capacity() * size_of::<Buffer<f64>>()
                + c.levels.iter().map(|v| v.values.capacity() * 8).sum::<usize>();
            assert_eq!(budget.used(), bytes);
        }
        all.finish().unwrap();
        let view = all.view().unwrap().clone();
        assert_eq!(view.metadata.samples, 21 * 999);
        assert_eq!(view.metadata.zeros, (21 * 999_u64).div_ceil(5));
        let queries: Vec<_> = (0..=1000).map(|i| view.quantile(i as f64 / 1000.0).unwrap()).collect();
        assert!(queries.windows(2).all(|w| w[0] <= w[1]));
        assert!(queries.iter().all(|v| v.is_finite()));
        drop(all);
        assert_eq!(
            budget.used(),
            size_of::<Quantiles>() + 2 * size_of::<usize>() + view.points.values.capacity() * 16
        );
        drop(view);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn failed_growth_merge_and_query_preparation_release_every_reservation() {
        const LIMIT: usize = 1 << 20;
        for operation in 0..3 {
            let budget = SummaryBudget::new(LIMIT, None);
            let mut a = Sketch::new(128, 1, &budget).unwrap();
            let mut b = Some(Sketch::new(128, 2, &budget).unwrap());
            fill(&mut a, 1, 16).unwrap();
            fill(b.as_mut().unwrap(), 30, 16).unwrap();
            let blocked = LIMIT - budget.used();
            let reservation = budget.reserve(blocked).unwrap();
            let error = match operation {
                0 => fill(&mut a, 100, 1000).unwrap_err(),
                1 => a.merge(*b.take().unwrap()).unwrap_err(),
                _ => a.finish().unwrap_err(),
            };
            assert_eq!(error.kind, crate::ErrorKind::Limit);
            drop(a);
            drop(b);
            assert_eq!(budget.used(), blocked);
            drop(reservation);
            assert_eq!(budget.used(), 0);
        }
    }

    #[test]
    fn cancellation_is_checked_for_updates_merges_and_finalization() {
        for operation in 0..3 {
            let flag = Arc::new(AtomicBool::new(false));
            let budget = SummaryBudget::new(1 << 20, Some(flag.clone()));
            let mut a = Sketch::new(128, 42, &budget).unwrap();
            let mut b = Some(Sketch::new(128, 43, &budget).unwrap());
            fill(&mut a, 0, 1000).unwrap();
            flag.store(true, Ordering::Relaxed);
            let error = match operation {
                0 => a.add(1.0).unwrap_err(),
                1 => a.merge(*b.take().unwrap()).unwrap_err(),
                _ => a.finish().unwrap_err(),
            };
            assert_eq!(error.message, "cancelled");
            drop(a);
            drop(b);
            assert_eq!(budget.used(), 0);
        }
    }

    #[test]
    fn zero_atoms_endpoints_extremes_and_ties_do_not_interpolate() {
        let budget = SummaryBudget::new(1 << 20, None);
        let mut s = Sketch::new(128, 42, &budget).unwrap();
        for v in [-f64::MAX, -f64::from_bits(1), -0.0, 0.0, f64::from_bits(1), f64::MAX] {
            s.add(v).unwrap();
        }
        s.finish().unwrap();
        let v = s.view().unwrap();
        assert_eq!(v.quantile(0.0), Some(-f64::MAX));
        assert_eq!(v.quantile(1.0), Some(f64::MAX));
        assert_eq!(v.quantile(0.5), Some(0.0));
        assert_eq!(v.quantile(0.666_666), Some(0.0));
        assert_eq!(v.quantile(0.666_667), Some(f64::from_bits(1)));
        for q in [f64::NAN, f64::INFINITY, -1.0, 1.1] {
            assert_eq!(v.quantile(q), None);
        }
        let mut s = Sketch::new(128, 42, &budget).unwrap();
        for _ in 0..10_000 {
            s.add(-0.0).unwrap();
        }
        s.finish().unwrap();
        assert!(s.view().unwrap().points.is_empty());
        assert_eq!(s.view().unwrap().metadata.zeros, 10_000);
        assert_eq!(s.view().unwrap().quantile(0.5), Some(0.0));
    }
}
