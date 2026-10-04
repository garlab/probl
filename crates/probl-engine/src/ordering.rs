//! Stable, fallible sorting. User comparators may fail or be inconsistent;
//! neither case may panic in Rust's sort implementation or hide an error.
use crate::dist::Budget;
use crate::error::OpResult;
use std::cmp::Ordering;

pub(crate) fn reserve_sort(len: usize, budget: &mut Budget) -> OpResult<()> {
    if len > 1 {
        budget.collection(len as u128)?;
        let passes = usize::BITS - (len - 1).leading_zeros();
        budget.work((len as u64).saturating_mul(u64::from(passes)))?;
    }
    Ok(())
}

/// Bottom-up merge sort has bounded work even for an inconsistent comparator.
/// On a tie, taking the left item preserves input order in either direction.
/// Errors stop comparisons immediately; callers sort their own immutable copy.
pub(crate) fn try_sort_by<T: Clone, E>(
    items: &mut [T],
    mut compare: impl FnMut(&T, &T) -> Result<Ordering, E>,
) -> Result<(), E> {
    if items.len() < 2 {
        return Ok(());
    }
    let mut buffer = items.to_vec();
    let mut width = 1;
    while width < items.len() {
        let mut start = 0;
        while start < items.len() {
            let mid = start.saturating_add(width).min(items.len());
            let end = mid.saturating_add(width).min(items.len());
            let (mut left, mut right) = (start, mid);
            for dest in &mut buffer[start..end] {
                if left < mid && (right == end || !compare(&items[left], &items[right])?.is_gt()) {
                    *dest = items[left].clone();
                    left += 1;
                } else {
                    *dest = items[right].clone();
                    right += 1;
                }
            }
            start = end;
        }
        items.clone_from_slice(&buffer);
        width = width.saturating_mul(2);
    }
    Ok(())
}
