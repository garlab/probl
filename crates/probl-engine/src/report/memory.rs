//! Live accounting for bounded report collectors, shared by sites and workers.
//! These are conservative storage charges, not a process/allocator RSS limit.
use super::{Acc, RunStat, Sums};
use crate::error::{OpError, OpResult};
use crate::value::Value;
use std::mem::size_of;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Clone, Debug)]
pub(crate) struct SummaryBudget(Arc<State>);

#[derive(Debug)]
struct State {
    limit: usize,
    used: AtomicUsize,
    cancel: Option<Arc<AtomicBool>>,
}

fn exceeded() -> OpError {
    OpError::limit(
        "moments reports exceeded max_report_summary_bytes; use fewer groups or threads, or raise the host limit",
    )
}

impl SummaryBudget {
    pub fn new(limit: usize, cancel: Option<Arc<AtomicBool>>) -> Self {
        Self(Arc::new(State {
            limit,
            used: AtomicUsize::new(0),
            cancel,
        }))
    }

    pub(super) fn reserve(&self, bytes: usize) -> OpResult<Reservation> {
        let mut reservation = Reservation {
            budget: Some(self.clone()),
            bytes: 0,
            runs: 0,
        };
        reservation.grow(bytes)?;
        Ok(reservation)
    }

    pub(super) fn group(&self, key: &Value) -> OpResult<Box<Reservation>> {
        // Allow for partially occupied B-tree nodes, their links, the boxed
        // reservation, and the two Boolean categories (including a temporary
        // vector while finishing a run). No numeric population is retained.
        let fixed = 24 * size_of::<(Value, Acc)>() + 256 + 8 * size_of::<(Value, Sums)>();
        let mut size = KeySize {
            bytes: fixed,
            limit: self.0.limit,
            cancel: self.0.cancel.as_deref(),
        };
        size.add(0)?;
        size.value(key, 0)?;
        Ok(Box::new(self.reserve(size.bytes)?))
    }
}

#[derive(Debug)]
pub(super) struct Reservation {
    budget: Option<SummaryBudget>,
    bytes: usize,
    runs: usize,
}

impl Clone for Reservation {
    fn clone(&self) -> Self {
        // Acc/Sink/Outcome clones are host snapshots, never execution buffers.
        // The execution limit does not account for copies made by callers.
        Self {
            budget: None,
            bytes: 0,
            runs: 0,
        }
    }
}

impl Reservation {
    // `try_update` requires a newer Rust than our MSRV (1.85).
    #[allow(deprecated)]
    fn grow(&mut self, bytes: usize) -> OpResult<()> {
        if let Some(budget) = &self.budget {
            budget
                .0
                .used
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                    used.checked_add(bytes).filter(|&n| n <= budget.0.limit)
                })
                .map_err(|_| exceeded())?;
            self.bytes += bytes;
        }
        Ok(())
    }

    fn shrink(&mut self, bytes: usize) {
        if let Some(budget) = &self.budget {
            debug_assert!(bytes <= self.bytes);
            self.bytes -= bytes;
            budget.0.used.fetch_sub(bytes, Ordering::Relaxed);
        }
    }

    /// Reserve a growing run map while its old allocation is still live.
    pub fn grow_runs(&mut self, capacity: usize) -> OpResult<usize> {
        let buckets = if capacity == 0 {
            4
        } else {
            capacity
                .checked_add(1)
                .and_then(usize::checked_next_power_of_two)
                .and_then(|n| n.checked_mul(2))
                .ok_or_else(exceeded)?
        };
        let bytes = table_bytes::<(u32, RunStat)>(buckets)?;
        self.grow(bytes)?;
        Ok(bytes)
    }

    pub fn finish_growth(&mut self, bytes: usize) {
        self.shrink(self.runs);
        self.runs = if self.budget.is_some() { bytes } else { 0 };
    }

    pub fn end_batch(&mut self) {
        self.shrink(self.runs);
        self.runs = 0;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.shrink(self.bytes);
    }
}

// HashMap's SwissTable allocation: power-of-two buckets, one control byte per
// bucket and room for the control group/alignment. Growth reservations include
// both old and new tables. Revisit these implementation-specific charges if
// the standard library changes its table layout.
fn table_bytes<T>(buckets: usize) -> OpResult<usize> {
    buckets
        .checked_mul(size_of::<T>() + 1)
        .and_then(|n| n.checked_add(64))
        .ok_or_else(exceeded)
}

pub(super) fn reach_bytes(entries: usize) -> OpResult<usize> {
    if entries == 0 {
        return Ok(0);
    }
    let buckets = entries
        .checked_mul(8)
        .and_then(|n| n.checked_add(6))
        .map(|n| n / 7)
        .and_then(usize::checked_next_power_of_two)
        .ok_or_else(exceeded)?
        .max(4);
    // Include a previous allocation during growth. Summing across groups
    // deliberately overcounts runs that occur in more than one group.
    table_bytes::<(u32, crate::Weight)>(buckets)?
        .checked_mul(2)
        .ok_or_else(exceeded)
}

struct KeySize<'a> {
    bytes: usize,
    limit: usize,
    cancel: Option<&'a AtomicBool>,
}

impl KeySize<'_> {
    fn add(&mut self, bytes: usize) -> OpResult<()> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .filter(|&n| n <= self.limit)
            .ok_or_else(exceeded)?;
        Ok(())
    }

    fn array<T>(&mut self, n: usize) -> OpResult<()> {
        self.add(n.checked_mul(size_of::<T>()).ok_or_else(exceeded)?)
    }

    fn string(&mut self, s: &str) -> OpResult<()> {
        self.add(64)?;
        self.add(s.len())
    }

    fn integer(&mut self, n: &probl_number::Integer) -> OpResult<()> {
        if n.to_i64().is_none() {
            // Arithmetic can shrink the magnitude without shrinking its limb
            // buffer. Use the hard integer ceiling, not the current bit count.
            self.add(128 + 2 * probl_number::MAX_INTEGER_BITS.div_ceil(8) as usize)?;
        }
        Ok(())
    }

    fn value(&mut self, value: &Value, depth: usize) -> OpResult<()> {
        if self.cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(OpError::limit("cancelled"));
        }
        if depth > 128 {
            return Err(OpError::unsupported(
                "moments report keys may nest at most 128 levels; use exact collection for deeper keys",
            ));
        }
        match value {
            Value::Dead
            | Value::Unit
            | Value::Bool(_)
            | Value::Float(_)
            | Value::Complex(_)
            | Value::Prob(_)
            | Value::Date(_) => {}
            Value::Int(n) => self.integer(n)?,
            Value::Range(a, b) => {
                self.integer(a)?;
                self.integer(b)?;
            }
            Value::Str(s) => self.string(s)?,
            Value::Enum(e) => {
                self.add(64)?;
                self.string(&e.name)?;
            }
            Value::List(xs) => {
                self.add(64)?;
                self.array::<Value>(xs.capacity())?;
                for x in xs.iter() {
                    self.value(x, depth + 1)?;
                }
            }
            Value::Map(xs) => {
                self.add(64)?;
                self.array::<(Value, Value)>(xs.len().checked_mul(11).ok_or_else(exceeded)?)?;
                self.array::<usize>(xs.len().checked_mul(12).ok_or_else(exceeded)?)?;
                for (k, v) in xs.iter() {
                    self.value(k, depth + 1)?;
                    self.value(v, depth + 1)?;
                }
            }
            Value::Bag(xs) => {
                self.add(64)?;
                self.array::<(Value, u64)>(xs.capacity())?;
                for (v, _) in xs.iter() {
                    self.value(v, depth + 1)?;
                }
            }
            Value::Record(r) => {
                self.add(64)?;
                if let Some(ty) = &r.ty {
                    self.string(ty)?;
                }
                self.array::<(Arc<str>, Value)>(r.fields.capacity())?;
                for (k, v) in &r.fields {
                    self.string(k)?;
                    self.value(v, depth + 1)?;
                }
            }
            // These can keep arbitrary laws or closure environments alive.
            // Diagnose them explicitly rather than charging just an Arc pointer.
            Value::Builtin(_)
            | Value::Closure(_)
            | Value::Dist(_)
            | Value::Continuous(_)
            | Value::Counts(_)
            | Value::Joint(_)
            | Value::Delayed(_)
            | Value::Analytic(_)
            | Value::Event(_) => {
                return Err(OpError::unsupported(
                    "moments report keys require plain data without functions or distributions; use exact collection for other keys",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Weight;
    use crate::report::{ReportCollection, Sink};

    fn sink(budget: &SummaryBudget) -> Sink {
        Sink::new(ReportCollection::Moments, 10_000).with_summary_budget(budget.clone())
    }

    fn used(budget: &SummaryBudget) -> usize {
        budget.0.used.load(Ordering::Relaxed)
    }

    #[test]
    fn live_charges_follow_active_pending_and_merged_batches() {
        let budget = SummaryBudget::new(8 * 1024 * 1024, None);
        let mut combined = sink(&budget);
        let mut batch = sink(&budget);
        for run in 0..1000 {
            batch
                .add(Value::Unit, &Value::Bool(run % 2 == 0), Weight::ONE, Some(run))
                .unwrap();
        }
        let active = used(&budget);
        batch.end_batch().unwrap();
        let finished = used(&budget);
        assert!(finished > 0 && active > 4 * finished);
        combined.absorb(batch);
        assert_eq!(used(&budget), finished, "moving a new group transfers its charge");
        let mut pending = sink(&budget);
        pending
            .add(Value::Unit, &Value::Bool(true), Weight::ONE, Some(1000))
            .unwrap();
        pending.end_batch().unwrap();
        assert_eq!(used(&budget), 2 * finished, "pending batches remain charged");
        combined.absorb(pending);
        assert_eq!(
            used(&budget),
            finished,
            "merging an existing group releases its duplicate"
        );
        let mut other_site = sink(&budget);
        other_site
            .add(Value::Unit, &Value::Float(1.0), Weight::ONE, Some(0))
            .unwrap();
        assert!(used(&budget) > 2 * finished, "sites share the same live budget");
        drop(combined);
        drop(other_site);
        assert_eq!(used(&budget), 0);
    }

    #[test]
    fn failed_growth_and_batch_finalization_release_their_reservations() {
        let reference = SummaryBudget::new(usize::MAX, None);
        let mut probe = sink(&reference);
        probe
            .add(Value::Unit, &Value::Float(1.0), Weight::ONE, Some(0))
            .unwrap();
        let first_run = used(&reference);
        let budget = SummaryBudget::new(first_run, None);
        let mut batch = sink(&budget);
        for run in 0..3 {
            batch
                .add(Value::Unit, &Value::Float(1.0), Weight::ONE, Some(run))
                .unwrap();
        }
        // The first table fits three runs. Repeated visits need no new storage.
        batch
            .add(Value::Unit, &Value::Float(2.0), Weight::ONE, Some(0))
            .unwrap();
        assert!(
            batch
                .add(Value::Unit, &Value::Float(3.0), Weight::ONE, Some(3))
                .is_err()
        );
        assert_eq!(used(&budget), first_run);
        assert!(batch.end_batch().is_err(), "reach bookkeeping is also budgeted");
        assert_eq!(used(&budget), first_run);
        drop(batch);
        assert_eq!(used(&budget), 0);
        // No partial charge from a failed key-size check or failed reservation.
        assert!(budget.group(&Value::Str("x".repeat(first_run).into())).is_err());
        assert_eq!(used(&budget), 0);
    }

    #[test]
    fn workers_cannot_each_spend_the_whole_budget() {
        let budget = SummaryBudget::new(128, None);
        let barrier = std::sync::Barrier::new(8);
        let successes = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let reservation = budget.reserve(64);
                    if reservation.is_ok() {
                        successes.fetch_add(1, Ordering::Relaxed);
                    }
                    barrier.wait();
                    assert_eq!(used(&budget), 128);
                    barrier.wait();
                    drop(reservation);
                });
            }
        });
        assert_eq!(successes.load(Ordering::Relaxed), 2);
        assert_eq!(used(&budget), 0);
        let budget = SummaryBudget::new(usize::MAX, None);
        let all = budget.reserve(usize::MAX).unwrap();
        assert!(budget.reserve(1).is_err(), "accounting cannot wrap");
        drop(all);
        assert_eq!(used(&budget), 0);
    }

    #[test]
    fn nested_and_shared_key_payloads_are_charged_per_group() {
        let budget = SummaryBudget::new(usize::MAX, None);
        let text = Value::Str("x".repeat(100_000).into());
        let key = Value::list(vec![text.clone(), text]);
        let a = budget.group(&key).unwrap();
        let bytes = used(&budget);
        assert!(bytes > 200_000);
        let b = budget.group(&key).unwrap();
        assert_eq!(used(&budget), 2 * bytes);
        drop((a, b));
        assert_eq!(used(&budget), 0);
        let mut deep = Value::Unit;
        for _ in 0..130 {
            deep = Value::list(vec![deep]);
        }
        assert!(budget.group(&deep).is_err());
        assert_eq!(used(&budget), 0);
    }

    #[test]
    fn key_accounting_observes_cancellation() {
        let cancel = Arc::new(AtomicBool::new(true));
        let budget = SummaryBudget::new(usize::MAX, Some(cancel));
        assert!(budget.group(&Value::Unit).unwrap_err().message.contains("cancelled"));
        assert_eq!(used(&budget), 0);
    }
}
