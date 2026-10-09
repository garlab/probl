//! Worlds, and merging the ones that have become identical.

use crate::analytic::Constraints;
use crate::error::RuntimeError;
use crate::value::Value;
use crate::weight::Weight;
use probl_sema::SlotSet;
use probl_sema::ir::SlotId;
use rustc_hash::{FxHashMap, FxHasher};
use std::collections::BTreeSet;
use std::collections::hash_map::Entry;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// A returned value, the weight of the world that returned it, and the
/// restrictions on analytic draws that its caller can see.
pub type Returned = (Value, Weight, Constraints);

/// One possible state of the program: the variables of the current frame,
/// and how likely it is (including every observation so far).
#[derive(Clone, Debug)]
pub struct World {
    pub slots: Vec<Value>,
    pub constraints: Constraints,
    /// Latents passed into this call: restrictions on these remain observable
    /// by its caller even after every local alias dies. Constant per frame.
    pub inherited: Arc<BTreeSet<u64>>,
    pub weight: Weight,
    /// When sampling, the run this world is (docs/semantics.md, section 14).
    pub run: u32,
    /// When enumerating, how many densities the weight includes: weights
    /// with different numbers are in different units, so they don't add up.
    pub densities: u32,
}

impl World {
    pub fn scaled(mut self, factor: f64) -> World {
        self.weight = self.weight.scale(factor);
        self
    }
}

pub fn total_weight(worlds: &[World]) -> Weight {
    Weight::sum(worlds.iter().map(|w| w.weight))
}

/// Worlds leaving a statement, grouped by how they left it.
#[derive(Debug, Default)]
pub struct Flow {
    /// Continue with the next statement.
    pub next: Vec<World>,
    pub broke: Vec<World>,
    pub continued: Vec<World>,
    /// Left the function: the returned value and the world's weight.
    pub returned: Vec<Returned>,
    /// Faulted inside a `try` that may catch the fault: each world as it
    /// was when its statement began, with its fault, on its way to the
    /// `catch`.
    pub faulted: Vec<(World, RuntimeError)>,
}

impl Flow {
    pub fn next(worlds: Vec<World>) -> Flow {
        Flow {
            next: worlds,
            ..Flow::default()
        }
    }

    pub fn join(&mut self, other: Flow) {
        self.next.extend(other.next);
        self.broke.extend(other.broke);
        self.continued.extend(other.continued);
        self.returned.extend(other.returned);
        self.faulted.extend(other.faulted);
    }
}

/// Clear these slots: nobody will read them again.
pub fn clear(worlds: &mut [World], slots: &[SlotId]) {
    if slots.is_empty() {
        return;
    }
    for w in worlds {
        for &s in slots {
            w.slots[s as usize] = Value::Dead;
        }
    }
}

/// Clear every slot that isn't live, in worlds that jumped (with `break` or
/// `continue`) past statements that would have cleared some of them.
pub fn clear_dead(worlds: &mut [World], live: &SlotSet) {
    let Some(n) = worlds.first().map(|w| w.slots.len()) else {
        return;
    };
    let dead: Vec<SlotId> = live.iter_missing(n).collect();
    clear(worlds, &dead);
}

/// The live slots among a frame's `n`.
pub fn live_slots(live: &SlotSet, n: usize) -> Vec<usize> {
    live.iter().map(|s| s as usize).filter(|&s| s < n).collect()
}

/// A hash of a world's live slots, as merging computes it.
pub fn state_hash(w: &World, live: &[usize]) -> u64 {
    let mut hasher = FxHasher::default();
    for &i in live {
        w.slots[i].hash(&mut hasher);
    }
    w.constraints.hash(&mut hasher);
    hasher.finish()
}

/// If `enabled`, merge the worlds that agree on the live slots, adding up
/// their weights, and keeping the order of first appearance. Dead slots
/// don't matter, cleared or not.
pub fn merge(mut worlds: Vec<World>, live: &SlotSet, enabled: bool) -> Vec<World> {
    for w in &mut worlds {
        if w.constraints.keys().all(|id| w.inherited.contains(id)) {
            continue;
        }
        let mut ids = (*w.inherited).clone();
        for slot in live.iter() {
            crate::analytic::collect_ids(&w.slots[slot as usize], &mut ids);
        }
        Arc::make_mut(&mut w.constraints).retain(|id, _| ids.contains(id));
    }
    if !enabled || worlds.len() < 2 {
        return worlds;
    }
    let n = worlds[0].slots.len();
    let live = live_slots(live, n);
    let mut out: Vec<World> = Vec::with_capacity(worlds.len());
    // The first world kept with each hash, and after each world, the next
    // one with the same hash (only when different worlds' hashes collide).
    let mut first: FxHashMap<u64, usize> = FxHashMap::with_capacity_and_hasher(worlds.len(), Default::default());
    let mut next: Vec<usize> = Vec::with_capacity(worlds.len());
    const NONE: usize = usize::MAX;
    for w in worlds {
        let same = |kept: &World| {
            kept.densities == w.densities
                && kept.constraints == w.constraints
                && live.iter().all(|&i| kept.slots[i] == w.slots[i])
        };
        let found = match first.entry(state_hash(&w, &live)) {
            Entry::Vacant(entry) => {
                entry.insert(out.len());
                None
            }
            Entry::Occupied(entry) => {
                let mut i = *entry.get();
                loop {
                    if same(&out[i]) {
                        break Some(i);
                    }
                    if next[i] == NONE {
                        next[i] = out.len();
                        break None;
                    }
                    i = next[i];
                }
            }
        };
        match found {
            Some(i) => out[i].weight += w.weight,
            None => {
                next.push(NONE);
                out.push(w);
            }
        }
    }
    out
}

/// Merge returned values only when their caller-visible restrictions also agree.
pub fn merge_values(pairs: Vec<Returned>) -> Vec<Returned> {
    let mut out: Vec<Returned> = Vec::with_capacity(pairs.len());
    let mut index: FxHashMap<(Value, Constraints), usize> = FxHashMap::default();
    for (v, w, constraints) in pairs {
        match index.entry((v.clone(), constraints.clone())) {
            Entry::Occupied(entry) => out[*entry.get()].1 += w,
            Entry::Vacant(entry) => {
                entry.insert(out.len());
                out.push((v, w, constraints));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(slots: Vec<Value>) -> World {
        World {
            slots,
            constraints: Default::default(),
            inherited: Default::default(),
            weight: Weight::ONE,
            run: 0,
            densities: 0,
        }
    }

    #[test]
    fn worlds_merge_on_their_live_slots() {
        let mut live = SlotSet::with_capacity(3);
        live.insert(0);
        // Slot 1 is dead: worlds that differ only there are the same world.
        let worlds = vec![
            world(vec![Value::Int(1.into()), Value::Int(7.into()), Value::Dead]),
            world(vec![Value::Int(2.into()), Value::Int(7.into()), Value::Dead]),
            world(vec![
                Value::Int(1.into()),
                Value::list(vec![Value::Int(8.into())]),
                Value::Dead,
            ]),
        ];
        let mut merged = merge(worlds, &live, true);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].slots[0], Value::Int(1.into()));
        assert_eq!(merged[0].weight.to_f64(), 2.0);
        assert_eq!(merged[1].weight.to_f64(), 1.0);
        clear_dead(&mut merged, &live);
        assert!(merged.iter().all(|w| w.slots[1] == Value::Dead));
        assert_eq!(merged[1].slots[0], Value::Int(2.into()));
    }
}
