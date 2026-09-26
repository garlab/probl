//! Worlds, and merging the ones that have become identical.

use crate::value::Value;
use probl_sema::SlotSet;
use rustc_hash::{FxHashMap, FxHasher};
use std::hash::{Hash, Hasher};

/// One possible state of the program: the variables of the current frame,
/// and how likely it is.
#[derive(Clone, Debug)]
pub struct World {
    pub slots: Vec<Value>,
    pub weight: f64,
}

impl World {
    pub fn scaled(mut self, factor: f64) -> World {
        self.weight *= factor;
        self
    }
}

pub fn total_weight(worlds: &[World]) -> f64 {
    worlds.iter().map(|w| w.weight).sum()
}

/// Worlds leaving a statement, grouped by how they left it.
#[derive(Debug, Default)]
pub struct Flow {
    /// Continue with the next statement.
    pub next: Vec<World>,
    pub broke: Vec<World>,
    pub continued: Vec<World>,
    /// Left the function: the returned value and the world's weight.
    pub returned: Vec<(Value, f64)>,
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
    }
}

/// Clear the slots that aren't live, then merge worlds that are now equal,
/// adding up their weights. The order of first appearance is kept.
pub fn merge(worlds: Vec<World>, live: &SlotSet, enabled: bool) -> Vec<World> {
    let mut worlds = worlds;
    for w in &mut worlds {
        for (i, slot) in w.slots.iter_mut().enumerate() {
            if !live.contains(i as u32) && !matches!(slot, Value::Dead) {
                *slot = Value::Dead;
            }
        }
    }
    if !enabled || worlds.len() < 2 {
        return worlds;
    }
    let mut out: Vec<World> = Vec::with_capacity(worlds.len());
    let mut index: FxHashMap<u64, Vec<usize>> = FxHashMap::default();
    for w in worlds {
        let mut hasher = FxHasher::default();
        w.slots.hash(&mut hasher);
        let h = hasher.finish();
        let bucket = index.entry(h).or_default();
        if let Some(&i) = bucket.iter().find(|&&i| out[i].slots == w.slots) {
            out[i].weight += w.weight;
        } else {
            bucket.push(out.len());
            out.push(w);
        }
    }
    out
}

/// Merge (value, weight) pairs with equal values.
pub fn merge_values(pairs: Vec<(Value, f64)>) -> Vec<(Value, f64)> {
    let mut out: Vec<(Value, f64)> = Vec::with_capacity(pairs.len());
    let mut index: FxHashMap<Value, usize> = FxHashMap::default();
    for (v, w) in pairs {
        match index.get(&v) {
            Some(&i) => out[i].1 += w,
            None => {
                index.insert(v.clone(), out.len());
                out.push((v, w));
            }
        }
    }
    out
}
