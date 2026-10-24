//! Live heap accounting for multivariate recipes, factors, caches and scratch.
use crate::error::{OpError, OpResult};
use std::mem::size_of;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Debug)]
pub struct Resources(Arc<State>);

#[derive(Debug)]
struct State {
    dimension: usize,
    limit: usize,
    used: AtomicUsize,
    peak: AtomicUsize,
}

fn allocation() -> OpError {
    OpError::limit("multivariate storage exceeded max_multivariate_bytes or could not be allocated")
}

impl Resources {
    pub fn new(dimension: usize, bytes: usize) -> Self {
        Self(Arc::new(State {
            dimension: dimension.min(super::MAX_DIMENSION),
            limit: bytes,
            used: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }))
    }

    pub fn used_bytes(&self) -> usize {
        self.0.used.load(Ordering::Relaxed)
    }

    pub fn peak_bytes(&self) -> usize {
        self.0.peak.load(Ordering::Relaxed)
    }

    pub(super) fn dimension(&self, n: usize) -> OpResult<()> {
        if n > self.0.dimension {
            return Err(OpError::limit(format!(
                "multivariate dimension {n} exceeds max_multivariate_dimension ({})",
                self.0.dimension
            )));
        }
        Ok(())
    }

    pub(super) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    #[allow(deprecated)] // fetch_update supports our Rust 1.85 MSRV.
    pub(super) fn reserve(&self, bytes: usize) -> OpResult<Reservation> {
        let old = self
            .0
            .used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(bytes).filter(|&new| new <= self.0.limit)
            })
            .map_err(|_| allocation())?;
        self.0.peak.fetch_max(old + bytes, Ordering::Relaxed);
        Ok(Reservation {
            resources: self.clone(),
            bytes,
        })
    }
}

#[derive(Debug)]
pub(super) struct Reservation {
    pub(super) resources: Resources,
    bytes: usize,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.resources.0.used.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub(super) struct Buffer<T> {
    // Drop the payload before releasing its charge. No detached/deep clones.
    values: Vec<T>,
    memory: Reservation,
}

impl<T> Buffer<T> {
    pub(super) fn empty(resources: &Resources) -> Self {
        Self {
            values: Vec::new(),
            memory: Reservation {
                resources: resources.clone(),
                bytes: 0,
            },
        }
    }

    pub(super) fn with_capacity(n: usize, resources: &Resources) -> OpResult<Self> {
        let memory = resources.reserve(n.checked_mul(size_of::<T>()).ok_or_else(allocation)?)?;
        let mut values = Vec::new();
        values.try_reserve_exact(n).map_err(|_| allocation())?;
        // Account requested capacity, excluding allocator rounding/metadata,
        // as for bounded report buffers. No amortized allocation here.
        Ok(Self { values, memory })
    }

    pub(super) fn filled(n: usize, value: T, resources: &Resources) -> OpResult<Self>
    where
        T: Clone,
    {
        let mut buffer = Self::with_capacity(n, resources)?;
        buffer.values.resize(n, value);
        Ok(buffer)
    }

    pub(super) fn grow(&mut self, n: usize) -> OpResult<()> {
        if n > self.values.capacity() {
            let mut new = Self::with_capacity(n, &self.memory.resources)?;
            new.values.append(&mut self.values);
            // Old and new allocations remain charged until replacement.
            *self = new;
        }
        Ok(())
    }

    pub(super) fn capacity(&self) -> usize {
        self.values.capacity()
    }

    pub(super) fn insert(&mut self, index: usize, value: T) {
        assert!(self.values.len() < self.values.capacity());
        self.values.insert(index, value);
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

impl<T> AsRef<[T]> for Buffer<T> {
    fn as_ref(&self) -> &[T] {
        &self.values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounting_and_capacity_arithmetic_cannot_wrap() {
        let resources = Resources::new(64, usize::MAX);
        let all = resources.reserve(usize::MAX).unwrap();
        assert!(resources.reserve(1).is_err());
        drop(all);
        assert_eq!(resources.used_bytes(), 0);
        assert!(Buffer::<f64>::with_capacity(usize::MAX, &resources).is_err());
        assert_eq!(resources.used_bytes(), 0);
        // Vec rejects this layout, after its accounting reservation succeeds.
        assert!(Buffer::<u8>::with_capacity(usize::MAX, &resources).is_err());
        assert_eq!(resources.used_bytes(), 0);
    }
}
