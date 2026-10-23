//! Experimental candidates, deliberately outside the engine and its public API.
//! See docs/quantile-sketches.md for the algorithms, limits and evaluation.
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// SplitMix64's integer mixer. Sketch streams never use the model's RNG.
pub fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[derive(Clone, Debug)]
pub struct Random(u64);

impl Random {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix(self.0)
    }
    pub fn unit(&mut self) -> f64 {
        // Strictly inside (0, 1), including after rounding.
        ((self.next() >> 12) as f64 + 0.5) / (1_u64 << 52) as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Compactor,
    Reservoir,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Compactor => "compactor",
            Self::Reservoir => "reservoir",
        }
    }
}

#[derive(Clone, Debug)]
enum Core {
    Compactor(Compactor),
    Reservoir(Reservoir),
}

/// Both candidates share the same exact endpoint and zero-mass treatment.
#[derive(Clone, Debug)]
pub struct Sketch {
    core: Core,
    pub count: u64,
    pub negative: u64,
    pub zeros: u64,
    min: f64,
    max: f64,
}

impl Sketch {
    pub fn new(kind: Kind, capacity: usize, seed: u64, batch: u64) -> Self {
        assert!((8..=65_536).contains(&capacity));
        Self {
            core: match kind {
                Kind::Compactor => Core::Compactor(Compactor::new(capacity, mix(seed ^ mix(batch)))),
                Kind::Reservoir => Core::Reservoir(Reservoir::new(capacity, seed)),
            },
            count: 0,
            negative: 0,
            zeros: 0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }

    pub fn add(&mut self, value: f64, id: u64) {
        assert!(value.is_finite());
        self.count = self.count.checked_add(1).expect("sample count overflow");
        let value = if value == 0.0 { 0.0 } else { value };
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.negative += u64::from(value < 0.0);
        self.zeros += u64::from(value == 0.0);
        if value != 0.0 {
            match &mut self.core {
                Core::Compactor(c) => c.add(value),
                Core::Reservoir(r) => r.add(Item {
                    priority: mix(id.wrapping_add(r.seed)),
                    id,
                    value,
                }),
            }
        }
    }

    /// Only disjoint inputs with matching settings may be merged.
    pub fn merge(&mut self, other: Self) {
        self.count = self.count.checked_add(other.count).expect("sample count overflow");
        self.negative += other.negative;
        self.zeros += other.zeros;
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
        match (&mut self.core, other.core) {
            (Core::Compactor(a), Core::Compactor(b)) => a.merge(b),
            (Core::Reservoir(a), Core::Reservoir(b)) => {
                assert_eq!((a.capacity, a.seed), (b.capacity, b.seed));
                for item in b.items {
                    a.add(item);
                }
            }
            _ => panic!("different sketch kinds"),
        }
    }

    pub fn retained(&self) -> usize {
        match &self.core {
            Core::Compactor(c) => c.size,
            Core::Reservoir(r) => r.items.len(),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut points: Vec<(f64, u64)> = match &self.core {
            Core::Compactor(c) => c
                .levels
                .iter()
                .enumerate()
                .flat_map(|(level, values)| values.iter().map(move |&v| (v, 1_u64 << level)))
                .collect(),
            Core::Reservoir(r) => r.items.iter().map(|i| (i.value, 1)).collect(),
        };
        points.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let split = points.partition_point(|&(v, _)| v < 0.0);
        let negative_weight = points[..split].iter().map(|p| p.1).sum();
        let positive_weight = points[split..].iter().map(|p| p.1).sum();
        Snapshot {
            points,
            split,
            negative_weight,
            positive_weight,
            count: self.count,
            negative: self.negative,
            zeros: self.zeros,
            min: self.min,
            max: self.max,
        }
    }
}

/// A sorted query view, with exact masses for negative / zero / positive values.
/// Reweight each nonzero side to its exact count, preserving the zero interval
/// even when compaction or reservoir sampling loses all values on a rare side.
#[derive(Debug)]
pub struct Snapshot {
    points: Vec<(f64, u64)>,
    split: usize,
    negative_weight: u64,
    positive_weight: u64,
    count: u64,
    negative: u64,
    zeros: u64,
    min: f64,
    max: f64,
}

impl Snapshot {
    /// Lower inverse empirical CDF, including at 0.5; no interpolation.
    /// This experiment does not change Probl's exact midpoint-median convention.
    pub fn quantile(&self, q: f64) -> Option<f64> {
        if self.count == 0 || !q.is_finite() || !(0.0..=1.0).contains(&q) {
            return None;
        }
        if q == 0.0 {
            return Some(self.min);
        }
        if q == 1.0 {
            return Some(self.max);
        }
        let target = (q * self.count as f64).ceil().max(1.0);
        let (points, weight, rank, count, fallback) = if target <= self.negative as f64 {
            (
                &self.points[..self.split],
                self.negative_weight,
                target,
                self.negative,
                self.min,
            )
        } else if target <= (self.negative + self.zeros) as f64 {
            return Some(0.0);
        } else {
            (
                &self.points[self.split..],
                self.positive_weight,
                target - (self.negative + self.zeros) as f64,
                self.count - self.negative - self.zeros,
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

/// A lazy hierarchy of randomized compactors, inspired by KLL. This is not
/// Apache DataSketches and does not inherit its empirical error calibration.
/// Integer capacity decay, independent coin flips and a minimum level size of
/// eight keep the prototype small and reproducible. No centroid arithmetic.
#[derive(Clone, Debug)]
struct Compactor {
    k: usize,
    levels: Vec<Vec<f64>>,
    size: usize,
    limit: usize,
    random: Random,
}

impl Compactor {
    fn new(k: usize, seed: u64) -> Self {
        Self {
            k,
            levels: vec![Vec::new()],
            size: 0,
            limit: k,
            random: Random::new(seed),
        }
    }

    fn capacity(&self, level: usize) -> usize {
        let mut capacity = self.k;
        for _ in level + 1..self.levels.len() {
            capacity = (2 * capacity).div_ceil(3).max(8);
        }
        capacity
    }

    fn add(&mut self, value: f64) {
        self.levels[0].push(value);
        self.size += 1;
        self.compress();
    }

    fn merge(&mut self, other: Self) {
        assert_eq!(self.k, other.k);
        self.levels
            .resize_with(self.levels.len().max(other.levels.len()), Vec::new);
        self.limit = (0..self.levels.len()).map(|i| self.capacity(i)).sum();
        self.size += other.size;
        for (i, mut values) in other.levels.into_iter().enumerate() {
            self.levels[i].append(&mut values);
        }
        self.compress();
    }

    fn compress(&mut self) {
        let mut compacted = false;
        while self.size >= self.limit {
            compacted = true;
            let level = (0..self.levels.len())
                .find(|&i| self.levels[i].len() >= self.capacity(i))
                .expect("some level must be full");
            if level + 1 == self.levels.len() {
                assert!(self.levels.len() < 64, "sample count overflow");
                self.levels.push(Vec::new());
                self.limit = (0..self.levels.len()).map(|i| self.capacity(i)).sum();
            }
            let (lower, upper) = self.levels.split_at_mut(level + 1);
            let values = &mut lower[level];
            values.sort_unstable_by(f64::total_cmp);
            // Preserve the largest item if odd; only compact an even prefix.
            // Either parity of that prefix is an unbiased rank estimator.
            let odd = (values.len() % 2 == 1).then(|| values.pop().unwrap());
            let parity = (self.random.next() & 1) as usize;
            upper[0].extend(values.iter().skip(parity).step_by(2).copied());
            self.size -= values.len() / 2;
            values.clear();
            values.extend(odd);
        }
        if compacted {
            // Lazy compaction temporarily lets a level borrow the hierarchy's
            // entire capacity. Keeping every old allocation after that level
            // empties would multiply live heap by the number of levels.
            for level in 0..self.levels.len() {
                let wanted = self.capacity(level).max(self.levels[level].len());
                if self.levels[level].capacity() > 2 * wanted {
                    self.levels[level].shrink_to(wanted);
                }
            }
        }
    }
}

/// Retain the k smallest deterministic per-observation priorities. Unlike
/// Algorithm R, these reservoirs merge without resampling or unequal weights.
#[derive(Clone, Debug)]
struct Reservoir {
    capacity: usize,
    seed: u64,
    items: BinaryHeap<Item>,
}

impl Reservoir {
    fn new(capacity: usize, seed: u64) -> Self {
        Self {
            capacity,
            seed: mix(seed ^ 0x5245_5345_5256_4f49),
            items: BinaryHeap::new(),
        }
    }
    fn add(&mut self, item: Item) {
        if self.items.len() < self.capacity {
            self.items.push(item);
        } else if item < *self.items.peek().unwrap() {
            *self.items.peek_mut().unwrap() = item;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Item {
    priority: u64,
    id: u64,
    value: f64,
}
impl PartialEq for Item {
    fn eq(&self, other: &Self) -> bool {
        (self.priority, self.id) == (other.priority, other.id)
    }
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Item {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.priority, self.id).cmp(&(other.priority, other.id))
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
