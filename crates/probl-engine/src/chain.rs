//! Solving loops as absorbing Markov chains (docs/semantics.md, section 10).
//!
//! A loop's states are its worlds at the loop's start, told apart by the
//! variables read later. Running the body once from each state gives the
//! chance of going on to each state, and of leaving the loop. The expected
//! number of times each state is visited, from the weights the loop starts
//! with, then says how much weight leaves by each way out.
//!
//! States are solved one strongly connected group at a time, in the order
//! weight flows through them. Within a group, states are eliminated one by
//! one (Grassmann, Taksar and Heyman, 1985). Every step adds nonnegative
//! numbers: the chance of leaving a state is the sum of its ways out, never
//! 1 minus the chance of staying, so a chain whose exits are rare loses no
//! precision.

use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// One step of a chain from each of its states.
#[derive(Clone, Debug, Default)]
pub struct Chain {
    /// For each state: the chance of going on to each state, itself
    /// included, each state at most once.
    pub next: Vec<Vec<(usize, f64)>>,
    /// For each state: the chance of leaving the chain in one step, by any
    /// way out.
    pub leave: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Solution {
    /// The expected number of visits to each state.
    Visits(Vec<f64>),
    /// A state from which the chain can never be left.
    Stuck(usize),
    /// Solving would take more steps than allowed.
    TooBig,
}

impl Chain {
    /// The expected number of visits to each state, starting from the
    /// chances in `start`, in at most `budget` elimination steps, which are
    /// taken from it. Every state must be reachable from `start`.
    pub fn visits(&self, start: &[f64], budget: &mut u64) -> Solution {
        let n = self.next.len();
        let mut inflow = start.to_vec();
        let mut visits = vec![0.0; n];
        // Tarjan's algorithm finds the groups in reverse order of flow.
        let mut groups = strongly_connected(&self.next);
        groups.reverse();
        let mut group_of = vec![0; n];
        for (g, states) in groups.iter().enumerate() {
            for &s in states {
                group_of[s] = g;
            }
        }
        for (g, states) in groups.iter().enumerate() {
            let solved = if let [s] = states[..] {
                // One state: it leaves to later groups, or comes back to itself.
                let away: f64 = self.next[s].iter().filter(|&&(j, _)| j != s).map(|(_, p)| p).sum();
                let d = self.leave[s] + away;
                if d <= 0.0 {
                    return Solution::Stuck(s);
                }
                visits[s] = inflow[s] / d;
                true
            } else {
                match self.eliminate(states, g, &group_of, &inflow, &mut visits, budget) {
                    Ok(()) => true,
                    Err(solution) => return solution,
                }
            };
            debug_assert!(solved);
            // What leaves the group flows into later ones.
            for &s in states {
                for &(j, p) in &self.next[s] {
                    if group_of[j] != g {
                        inflow[j] += visits[s] * p;
                    }
                }
            }
        }
        if visits.iter().any(|v| !v.is_finite()) {
            return Solution::TooBig;
        }
        Solution::Visits(visits)
    }

    /// Solve one strongly connected group by eliminating its states, fewest
    /// connections first.
    fn eliminate(
        &self,
        states: &[usize],
        group: usize,
        group_of: &[usize],
        inflow: &[f64],
        visits: &mut [f64],
        budget: &mut u64,
    ) -> Result<(), Solution> {
        let m = states.len();
        let local: FxHashMap<usize, usize> = states.iter().enumerate().map(|(i, &s)| (s, i)).collect();
        // Within the group: steps to its states, and the chance of leaving
        // it, including to later groups.
        let mut out: Vec<FxHashMap<usize, f64>> = vec![FxHashMap::default(); m];
        let mut into: Vec<FxHashSet<usize>> = vec![FxHashSet::default(); m];
        let mut leave = vec![0.0; m];
        let mut mass = vec![0.0; m];
        for (i, &s) in states.iter().enumerate() {
            leave[i] = self.leave[s];
            mass[i] = inflow[s];
            for &(j, p) in &self.next[s] {
                if group_of[j] == group {
                    let j = local[&j];
                    *out[i].entry(j).or_insert(0.0) += p;
                    if j != i {
                        into[j].insert(i);
                    }
                } else {
                    leave[i] += p;
                }
            }
        }
        let degree = |i: usize, out: &[FxHashMap<usize, f64>], into: &[FxHashSet<usize>]| {
            (into[i].len() as u64) * (out[i].len() as u64)
        };
        let mut queue: BinaryHeap<Reverse<(u64, usize)>> =
            (0..m).map(|i| Reverse((degree(i, &out, &into), i))).collect();
        let mut alive = vec![true; m];
        // Each state eliminated, in order, to work out the visits to them
        // afterwards in reverse order.
        let mut steps: Vec<Eliminated> = Vec::with_capacity(m);
        while let Some(Reverse((d, k))) = queue.pop() {
            if !alive[k] || d != degree(k, &out, &into) {
                continue;
            }
            alive[k] = false;
            // Stays at k don't count: they only make each visit longer.
            out[k].remove(&k);
            let away: f64 = out[k].values().sum();
            let d_k = leave[k] + away;
            if d_k <= 0.0 {
                return Err(Solution::Stuck(states[k]));
            }
            let succs: Vec<(usize, f64)> = out[k].iter().map(|(&j, &p)| (j, p)).collect();
            let preds: Vec<(usize, f64)> = into[k]
                .iter()
                .map(|&i| (i, out[i].remove(&k).expect("an edge into k")))
                .collect();
            let cost = (preds.len() * succs.len() + 1) as u64;
            if cost > *budget {
                return Err(Solution::TooBig);
            }
            *budget -= cost;
            // Paths through k: i → k, then k's stays, then k → j or out.
            for &(i, p_ik) in &preds {
                let f = p_ik / d_k;
                for &(j, p_kj) in &succs {
                    *out[i].entry(j).or_insert(0.0) += f * p_kj;
                    if j != i {
                        into[j].insert(i);
                    }
                }
                leave[i] += f * leave[k];
            }
            // Weight starting at k goes on to its successors.
            for &(j, p_kj) in &succs {
                mass[j] += mass[k] * p_kj / d_k;
                into[j].remove(&k);
            }
            for &(i, _) in &preds {
                queue.push(Reverse((degree(i, &out, &into), i)));
            }
            for &(j, _) in &succs {
                queue.push(Reverse((degree(j, &out, &into), j)));
            }
            steps.push(Eliminated {
                state: k,
                mass: mass[k],
                leaving: d_k,
                from: preds,
            });
        }
        // Visits to k: the weight that starts there or arrives from a state
        // eliminated after it, times the expected stays, 1 / d_k.
        let mut local_visits = vec![0.0; m];
        for step in steps.into_iter().rev() {
            let arriving: f64 = step.from.iter().map(|&(i, p)| local_visits[i] * p).sum();
            local_visits[step.state] = (step.mass + arriving) / step.leaving;
        }
        for (i, &s) in states.iter().enumerate() {
            visits[s] = local_visits[i];
        }
        Ok(())
    }
}

/// A state as it was eliminated: what the visits to it are made of.
struct Eliminated {
    state: usize,
    /// The weight starting there, including what reached it through states
    /// eliminated before it.
    mass: f64,
    /// The chance of leaving it for another state, or out, each visit.
    leaving: f64,
    /// The states still there that could reach it, and with what chance.
    from: Vec<(usize, f64)>,
}

/// The strongly connected groups of states, each group's states in no
/// particular order, and the groups in reverse topological order (Tarjan's
/// algorithm, without recursion).
fn strongly_connected(next: &[Vec<(usize, f64)>]) -> Vec<Vec<usize>> {
    const UNSEEN: usize = usize::MAX;
    let n = next.len();
    let mut index = vec![UNSEEN; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut groups = Vec::new();
    let mut counter = 0;
    // (state, position in its list of successors)
    let mut calls: Vec<(usize, usize)> = Vec::new();
    for root in 0..n {
        if index[root] != UNSEEN {
            continue;
        }
        calls.push((root, 0));
        while let Some(&mut (v, ref mut at)) = calls.last_mut() {
            if *at == 0 {
                index[v] = counter;
                low[v] = counter;
                counter += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            if let Some(&(w, _)) = next[v].get(*at) {
                *at += 1;
                if index[w] == UNSEEN {
                    calls.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(parent, _)) = calls.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut group = Vec::new();
                loop {
                    let w = stack.pop().expect("v is on the stack");
                    on_stack[w] = false;
                    group.push(w);
                    if w == v {
                        break;
                    }
                }
                groups.push(group);
            }
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol * b.abs().max(1.0), "{a} vs {b}");
    }

    fn visits(chain: &Chain, start: &[f64]) -> Vec<f64> {
        match chain.visits(start, &mut u64::MAX.clone()) {
            Solution::Visits(v) => v,
            other => panic!("{other:?}"),
        }
    }

    /// The expected visits by summing the chain's steps until they vanish.
    fn by_iterating(chain: &Chain, start: &[f64]) -> Vec<f64> {
        let n = chain.next.len();
        let (mut total, mut now) = (start.to_vec(), start.to_vec());
        for _ in 0..200_000 {
            let mut after = vec![0.0; n];
            for (i, row) in chain.next.iter().enumerate() {
                for &(j, p) in row {
                    after[j] += now[i] * p;
                }
            }
            if after.iter().sum::<f64>() < 1e-18 {
                break;
            }
            for (t, a) in total.iter_mut().zip(&after) {
                *t += a;
            }
            now = after;
        }
        total
    }

    #[test]
    fn one_state_that_stays() {
        let chain = Chain {
            next: vec![vec![(0, 0.75)]],
            leave: vec![0.25],
        };
        assert_eq!(visits(&chain, &[1.0]), [4.0]);
    }

    #[test]
    fn gamblers_ruin_is_exact() {
        // A fair walk on 1..=9, leaving at 0 or 10: from k, the chance of
        // reaching 10 is k / 10, and the expected visits to j from k are
        // 2 min(j, k) (10 − max(j, k)) / 10.
        let n = 9;
        let next: Vec<Vec<(usize, f64)>> = (0..n)
            .map(|i| {
                let mut row = Vec::new();
                if i > 0 {
                    row.push((i - 1, 0.5));
                }
                if i + 1 < n {
                    row.push((i + 1, 0.5));
                }
                row
            })
            .collect();
        let leave = (0..n).map(|i| if i == 0 || i == n - 1 { 0.5 } else { 0.0 }).collect();
        let chain = Chain { next, leave };
        for k in 1..=9 {
            let mut start = vec![0.0; n];
            start[k - 1] = 1.0;
            let v = visits(&chain, &start);
            for j in 1..=9 {
                let expected = 2.0 * (j.min(k) * (10 - j.max(k))) as f64 / 10.0;
                close(v[j - 1], expected, 1e-13);
            }
            // Reaching 10 is leaving from 9.
            close(v[n - 1] * 0.5, k as f64 / 10.0, 1e-13);
        }
    }

    #[test]
    fn rare_exits_lose_no_precision() {
        // Two states that pass the weight back and forth, leaving with a
        // chance of 1e-15 each step: 1e15 visits, which 1 − (chance of
        // staying) couldn't compute.
        let chain = Chain {
            next: vec![vec![(1, 1.0 - 1e-15)], vec![(0, 1.0)]],
            leave: vec![1e-15, 0.0],
        };
        let v = visits(&chain, &[1.0, 0.0]);
        close(v[0], 1e15, 1e-12);
        close(v[0] * 1e-15, 1.0, 1e-12);
    }

    #[test]
    fn chains_that_cannot_be_left_are_found() {
        // 0 leads to the cycle 1 ⇄ 2, which never leaves.
        let chain = Chain {
            next: vec![vec![(1, 0.5)], vec![(2, 1.0)], vec![(1, 1.0)]],
            leave: vec![0.5, 0.0, 0.0],
        };
        assert!(matches!(
            chain.visits(&[1.0, 0.0, 0.0], &mut u64::MAX.clone()),
            Solution::Stuck(1 | 2)
        ));
        let chain = Chain {
            next: vec![vec![(0, 1.0)]],
            leave: vec![0.0],
        };
        assert_eq!(chain.visits(&[1.0], &mut u64::MAX.clone()), Solution::Stuck(0));
    }

    #[test]
    fn a_budget_stops_large_eliminations() {
        let n = 50;
        let next: Vec<Vec<(usize, f64)>> = (0..n)
            .map(|i| (0..n).map(|j| (j, 0.9 / n as f64)).filter(|&(j, _)| j != i).collect())
            .collect();
        let chain = Chain {
            next,
            leave: vec![0.1 + 0.9 / 50.0; n],
        };
        let mut start = vec![0.0; n];
        start[0] = 1.0;
        assert_eq!(chain.visits(&start, &mut 1000), Solution::TooBig);
        assert!(matches!(
            chain.visits(&start, &mut u64::MAX.clone()),
            Solution::Visits(_)
        ));
    }

    /// Random chains with several groups, cycles and self-loops, against
    /// summing their steps.
    #[test]
    fn random_chains_agree_with_iterating() {
        let mut rng = crate::continuous::Rng::new(5);
        for _ in 0..200 {
            let n = 1 + (rng.uniform() * 12.0) as usize;
            let mut next = Vec::new();
            let mut leave = Vec::new();
            for _ in 0..n {
                let mut row: Vec<(usize, f64)> = Vec::new();
                let mut weights = Vec::new();
                for j in 0..n {
                    if rng.uniform() < 0.3 {
                        row.push((j, 0.0));
                        weights.push(rng.uniform());
                    }
                }
                let out = rng.uniform() * 0.5 + 0.05;
                weights.push(out);
                let total: f64 = weights.iter().sum();
                for ((_, p), w) in row.iter_mut().zip(&weights) {
                    *p = w / total;
                }
                next.push(row);
                leave.push(out / total);
            }
            let chain = Chain { next, leave };
            let start: Vec<f64> = (0..n).map(|_| rng.uniform()).collect();
            let solved = visits(&chain, &start);
            let iterated = by_iterating(&chain, &start);
            for (a, b) in solved.iter().zip(&iterated) {
                close(*a, *b, 1e-9);
            }
        }
    }
}
