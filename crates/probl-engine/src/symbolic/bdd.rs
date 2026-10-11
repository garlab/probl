//! Reduced ordered binary decision diagrams with complement edges, and
//! weighted model counting over them.
//!
//! A diagram is an edge: a node's index with a complement bit. The only
//! terminal is `true`, and `false` is its complement. A node's high edge is
//! never complemented, which keeps diagrams canonical: two edges are equal
//! exactly when their functions are. Variables are ordered by when they
//! were made, the first at the top.
//!
//! Nodes are never reclaimed: a diagram lives as long as its manager, which
//! lives for one run. The number of nodes is bounded, and so is the run.

use crate::weight::Weight;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A Boolean function of the manager's variables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bdd(u32);

impl Bdd {
    pub const TRUE: Bdd = Bdd(0);
    pub const FALSE: Bdd = Bdd(1);

    fn index(self) -> usize {
        (self.0 >> 1) as usize
    }

    fn complemented(self) -> bool {
        self.0 & 1 != 0
    }

    fn regular(self) -> Bdd {
        Bdd(self.0 & !1)
    }

    #[must_use]
    pub fn not(self) -> Bdd {
        Bdd(self.0 ^ 1)
    }
}

#[derive(Clone, Copy)]
struct Node {
    var: u32,
    low: Bdd,
    high: Bdd,
}

/// The terminal's variable, below every other.
const TERMINAL: u32 = u32::MAX;
const EMPTY: u32 = u32::MAX;

/// Why a diagram couldn't be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// It would have more nodes than allowed.
    TooLarge,
    Cancelled,
}

/// An operation in the cache.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum Op {
    None,
    And,
    Xor,
    Ite,
}

/// A remembered result. The cache keeps one per slot, and a newer result
/// replaces an older one.
#[derive(Clone, Copy)]
struct Entry {
    op: Op,
    a: u32,
    b: u32,
    c: u32,
    result: Bdd,
}

const NO_ENTRY: Entry = Entry {
    op: Op::None,
    a: 0,
    b: 0,
    c: 0,
    result: Bdd::FALSE,
};

/// The largest cache, in entries (20 bytes each).
const MAX_CACHE: usize = 1 << 22;

/// Nodes made between checks for cancellation.
const CANCEL_CHECK: usize = 1 << 14;

/// The nodes of a run's diagrams, and the variables they're over.
pub struct Manager {
    nodes: Vec<Node>,
    /// Open addressing by (var, low, high): node indices, or `EMPTY`.
    unique: Vec<u32>,
    cache: Vec<Entry>,
    vars: u32,
    max_nodes: usize,
    cancel: Option<Arc<AtomicBool>>,
    /// Once set, operations return junk, and the run stops at the next
    /// check.
    stopped: Option<Stop>,
}

fn hash(a: u32, b: u32, c: u32) -> usize {
    let h = (a as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (b as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f)
        ^ (c as u64).wrapping_mul(0x1656_67b1_9e37_79f9);
    (h ^ (h >> 31)) as usize
}

impl Manager {
    /// A manager that makes at most `max_nodes` nodes, and stops when
    /// `cancel` is set.
    pub fn new(max_nodes: usize, cancel: Option<Arc<AtomicBool>>) -> Manager {
        Manager {
            nodes: vec![Node {
                var: TERMINAL,
                low: Bdd::TRUE,
                high: Bdd::TRUE,
            }],
            unique: vec![EMPTY; 1 << 12],
            cache: vec![NO_ENTRY; 1 << 12],
            vars: 0,
            // Indices take 31 bits.
            max_nodes: max_nodes.clamp(1, 1 << 31),
            cancel,
            stopped: None,
        }
    }

    /// Whether building stopped, and why.
    pub fn stopped(&self) -> Option<Stop> {
        self.stopped
    }

    /// The nodes made, besides the terminal.
    pub fn nodes(&self) -> usize {
        self.nodes.len() - 1
    }

    pub fn vars(&self) -> u32 {
        self.vars
    }

    /// A new variable, below the ones made before it.
    pub fn var(&mut self) -> Bdd {
        let v = self.vars;
        self.vars += 1;
        self.node(v, Bdd::FALSE, Bdd::TRUE)
    }

    fn var_of(&self, f: Bdd) -> u32 {
        self.nodes[f.index()].var
    }

    /// `f` with `var` set to false, and to true.
    fn cofactors(&self, f: Bdd, var: u32) -> (Bdd, Bdd) {
        let n = self.nodes[f.index()];
        if n.var != var {
            (f, f)
        } else if f.complemented() {
            (n.low.not(), n.high.not())
        } else {
            (n.low, n.high)
        }
    }

    /// The node `var ? high : low`, made once.
    fn node(&mut self, var: u32, low: Bdd, high: Bdd) -> Bdd {
        if low == high {
            return low;
        }
        // The high edge stays regular: ¬(v ? h : l) = v ? ¬h : ¬l.
        if high.complemented() {
            return self.node(var, low.not(), high.not()).not();
        }
        let mask = self.unique.len() - 1;
        let mut i = hash(var, low.0, high.0) & mask;
        loop {
            let at = self.unique[i];
            if at == EMPTY {
                break;
            }
            let n = self.nodes[at as usize];
            if n.var == var && n.low == low && n.high == high {
                return Bdd(at << 1);
            }
            i = (i + 1) & mask;
        }
        if self.nodes.len() > self.max_nodes {
            self.stopped = Some(Stop::TooLarge);
            return Bdd::FALSE;
        }
        if self.nodes.len() % CANCEL_CHECK == 0 && self.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
            self.stopped = Some(Stop::Cancelled);
            return Bdd::FALSE;
        }
        let index = self.nodes.len() as u32;
        self.nodes.push(Node { var, low, high });
        self.unique[i] = index;
        if self.nodes.len() * 2 > self.unique.len() {
            self.grow();
        }
        Bdd(index << 1)
    }

    /// Double the unique table, and the cache while it's smaller than the
    /// nodes.
    #[cold]
    fn grow(&mut self) {
        let size = self.unique.len() * 2;
        let mask = size - 1;
        let mut unique = vec![EMPTY; size];
        for (index, n) in self.nodes.iter().enumerate().skip(1) {
            let mut i = hash(n.var, n.low.0, n.high.0) & mask;
            while unique[i] != EMPTY {
                i = (i + 1) & mask;
            }
            unique[i] = index as u32;
        }
        self.unique = unique;
        if self.cache.len() < self.nodes.len().min(MAX_CACHE) {
            self.cache = vec![NO_ENTRY; (self.cache.len() * 4).min(MAX_CACHE)];
        }
    }

    fn cached(&self, op: Op, a: u32, b: u32, c: u32) -> Option<Bdd> {
        let e = self.cache[(hash(a, b, c) ^ op as usize) & (self.cache.len() - 1)];
        (e.op == op && e.a == a && e.b == b && e.c == c).then_some(e.result)
    }

    fn remember(&mut self, op: Op, a: u32, b: u32, c: u32, result: Bdd) {
        let i = (hash(a, b, c) ^ op as usize) & (self.cache.len() - 1);
        self.cache[i] = Entry { op, a, b, c, result };
    }

    pub fn and(&mut self, f: Bdd, g: Bdd) -> Bdd {
        if f == Bdd::FALSE || g == Bdd::FALSE || f == g.not() {
            return Bdd::FALSE;
        }
        if f == Bdd::TRUE || f == g {
            return g;
        }
        if g == Bdd::TRUE {
            return f;
        }
        if self.stopped.is_some() {
            return Bdd::FALSE;
        }
        let (f, g) = if f.0 <= g.0 { (f, g) } else { (g, f) };
        if let Some(r) = self.cached(Op::And, f.0, g.0, 0) {
            return r;
        }
        let v = self.var_of(f).min(self.var_of(g));
        let (f0, f1) = self.cofactors(f, v);
        let (g0, g1) = self.cofactors(g, v);
        let low = self.and(f0, g0);
        let high = self.and(f1, g1);
        let r = self.node(v, low, high);
        self.remember(Op::And, f.0, g.0, 0, r);
        r
    }

    pub fn or(&mut self, f: Bdd, g: Bdd) -> Bdd {
        self.and(f.not(), g.not()).not()
    }

    pub fn xor(&mut self, f: Bdd, g: Bdd) -> Bdd {
        // ¬f ⊕ g = ¬(f ⊕ g): the complements come out.
        let flip = f.complemented() != g.complemented();
        let r = self.xor_regular(f.regular(), g.regular());
        if flip { r.not() } else { r }
    }

    fn xor_regular(&mut self, f: Bdd, g: Bdd) -> Bdd {
        if f == g {
            return Bdd::FALSE;
        }
        if f == Bdd::TRUE {
            return g.not();
        }
        if g == Bdd::TRUE {
            return f.not();
        }
        if self.stopped.is_some() {
            return Bdd::FALSE;
        }
        let (f, g) = if f.0 <= g.0 { (f, g) } else { (g, f) };
        if let Some(r) = self.cached(Op::Xor, f.0, g.0, 0) {
            return r;
        }
        let v = self.var_of(f).min(self.var_of(g));
        let (f0, f1) = self.cofactors(f, v);
        let (g0, g1) = self.cofactors(g, v);
        let low = self.xor(f0, g0);
        let high = self.xor(f1, g1);
        let r = self.node(v, low, high);
        self.remember(Op::Xor, f.0, g.0, 0, r);
        r
    }

    /// `c ? t : e`.
    pub fn ite(&mut self, c: Bdd, t: Bdd, e: Bdd) -> Bdd {
        if c == Bdd::TRUE || t == e {
            return t;
        }
        if c == Bdd::FALSE {
            return e;
        }
        // (c ∧ t) ∨ (¬c ∧ e), when t or e is a constant or c itself.
        if t == Bdd::TRUE || t == c {
            return self.or(c, e);
        }
        if t == Bdd::FALSE || t == c.not() {
            return self.and(c.not(), e);
        }
        if e == Bdd::FALSE || e == c {
            return self.and(c, t);
        }
        if e == Bdd::TRUE || e == c.not() {
            return self.or(c.not(), t);
        }
        if self.stopped.is_some() {
            return Bdd::FALSE;
        }
        // A regular condition, and a regular `t`: ¬c ? t : e = c ? e : t,
        // and c ? ¬t : ¬e = ¬(c ? t : e).
        let (c, t, e) = if c.complemented() { (c.not(), e, t) } else { (c, t, e) };
        let flip = t.complemented();
        let (t, e) = if flip { (t.not(), e.not()) } else { (t, e) };
        let r = match self.cached(Op::Ite, c.0, t.0, e.0) {
            Some(r) => r,
            None => {
                let v = self.var_of(c).min(self.var_of(t)).min(self.var_of(e));
                let (c0, c1) = self.cofactors(c, v);
                let (t0, t1) = self.cofactors(t, v);
                let (e0, e1) = self.cofactors(e, v);
                let low = self.ite(c0, t0, e0);
                let high = self.ite(c1, t1, e1);
                let r = self.node(v, low, high);
                self.remember(Op::Ite, c.0, t.0, e.0, r);
                r
            }
        };
        if flip { r.not() } else { r }
    }

    /// The weighted model counts of `f` and of `¬f`: over the assignments
    /// of the variables that make it true (false), the sum of the products
    /// of their weights. `weights[v]` is variable `v`'s weight when false
    /// and when true, which add up to one. Counts of nodes go in `memo`,
    /// to be shared by later counts in the same diagrams.
    ///
    /// Neither count is computed from the other, so both keep their
    /// relative precision when one is tiny.
    pub fn count(&self, f: Bdd, weights: &[(Weight, Weight)], memo: &mut Counts) -> (Weight, Weight) {
        let (yes, no) = self.count_node(f.index() as u32, weights, memo);
        if f.complemented() { (no, yes) } else { (yes, no) }
    }

    fn count_node(&self, index: u32, weights: &[(Weight, Weight)], memo: &mut Counts) -> (Weight, Weight) {
        if index == 0 {
            return (Weight::ONE, Weight::ZERO);
        }
        if let Some(&c) = memo.0.get(&index) {
            return c;
        }
        let n = self.nodes[index as usize];
        let (w0, w1) = weights[n.var as usize];
        let low = self.count(n.low, weights, memo);
        let high = self.count(n.high, weights, memo);
        let c = (w0 * low.0 + w1 * high.0, w0 * low.1 + w1 * high.1);
        memo.0.insert(index, c);
        c
    }

    /// The weighted model count of `f ∧ g`, without making its nodes: a
    /// report's worlds are counted, and never needed again. `pairs` holds
    /// the counts of the pairs of nodes on the way, for this count only.
    pub fn count_and(
        &self,
        f: Bdd,
        g: Bdd,
        weights: &[(Weight, Weight)],
        memo: &mut Counts,
        pairs: &mut FxHashMap<(Bdd, Bdd), Weight>,
    ) -> Weight {
        if f == Bdd::FALSE || g == Bdd::FALSE || f == g.not() {
            return Weight::ZERO;
        }
        if f == Bdd::TRUE || f == g {
            return self.count(g, weights, memo).0;
        }
        if g == Bdd::TRUE {
            return self.count(f, weights, memo).0;
        }
        let (f, g) = if f.0 <= g.0 { (f, g) } else { (g, f) };
        if let Some(&c) = pairs.get(&(f, g)) {
            return c;
        }
        let v = self.var_of(f).min(self.var_of(g));
        let (f0, f1) = self.cofactors(f, v);
        let (g0, g1) = self.cofactors(g, v);
        let (w0, w1) = weights[v as usize];
        let low = self.count_and(f0, g0, weights, memo, pairs);
        let high = self.count_and(f1, g1, weights, memo, pairs);
        let c = w0 * low + w1 * high;
        pairs.insert((f, g), c);
        c
    }
}

/// Model counts of nodes, shared by the counts of one run.
#[derive(Default)]
pub struct Counts(FxHashMap<u32, (Weight, Weight)>);

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> Manager {
        Manager::new(1 << 20, None)
    }

    /// The truth table of `f` over the first `n` variables.
    fn table(m: &Manager, f: Bdd, n: u32) -> Vec<bool> {
        (0..1u32 << n)
            .map(|bits| {
                let mut e = f;
                loop {
                    let node = m.nodes[e.index()];
                    if node.var == TERMINAL {
                        break !e.complemented();
                    }
                    let next = if bits >> node.var & 1 == 1 { node.high } else { node.low };
                    e = if e.complemented() { next.not() } else { next };
                }
            })
            .collect()
    }

    #[test]
    fn operations_match_their_truth_tables() {
        let mut m = manager();
        let x: Vec<Bdd> = (0..4).map(|_| m.var()).collect();
        let bit = |bits: u32, i: usize| bits >> i & 1 == 1;
        let a = m.and(x[0], x[2].not());
        let b = m.xor(x[1], x[3]);
        let c = m.or(a, b);
        let d = m.ite(x[2], c, x[0].not());
        for bits in 0..16u32 {
            let (a_, b_) = (bit(bits, 0) && !bit(bits, 2), bit(bits, 1) != bit(bits, 3));
            let c_ = a_ || b_;
            let d_ = if bit(bits, 2) { c_ } else { !bit(bits, 0) };
            assert_eq!(table(&m, a, 4)[bits as usize], a_);
            assert_eq!(table(&m, b, 4)[bits as usize], b_);
            assert_eq!(table(&m, c, 4)[bits as usize], c_);
            assert_eq!(table(&m, d, 4)[bits as usize], d_);
        }
    }

    #[test]
    fn equal_functions_are_equal_diagrams() {
        let mut m = manager();
        let (x, y, z) = (m.var(), m.var(), m.var());
        // De Morgan, and xor written two ways.
        let a = m.and(x, y);
        let b = m.or(x.not(), y.not()).not();
        assert_eq!(a, b);
        let xy = m.xor(x, y);
        let both = m.and(x, y.not());
        let other = m.and(x.not(), y);
        assert_eq!(xy, m.or(both, other));
        // ite against its definition.
        let i = m.ite(x, y, z);
        let t = m.and(x, y);
        let e = m.and(x.not(), z);
        assert_eq!(i, m.or(t, e));
        assert_eq!(m.ite(x.not(), y, z), m.ite(x, z, y));
        assert_eq!(m.ite(x, y.not(), z.not()), i.not());
        assert_eq!(m.and(x, x.not()), Bdd::FALSE);
        assert_eq!(m.or(x, x.not()), Bdd::TRUE);
    }

    #[test]
    fn counts_weigh_assignments() {
        let mut m = manager();
        let (x, y) = (m.var(), m.var());
        let w = |p: f64| (Weight::new(1.0 - p), Weight::new(p));
        let weights = [w(0.3), w(0.6)];
        let mut memo = Counts::default();
        let f = m.or(x, y);
        let (yes, no) = m.count(f, &weights, &mut memo);
        assert!((yes.to_f64() - (1.0 - 0.7 * 0.4)).abs() < 1e-15);
        assert!((no.to_f64() - 0.7 * 0.4).abs() < 1e-15);
        assert_eq!(m.count(Bdd::TRUE, &weights, &mut memo), (Weight::ONE, Weight::ZERO));
        // A tiny count isn't one minus a count near one.
        let mut m = manager();
        let vars: Vec<Bdd> = (0..40).map(|_| m.var()).collect();
        let all = vars.iter().fold(Bdd::TRUE, |acc, &v| m.and(acc, v));
        let weights = vec![w(1e-10); 40];
        let (yes, no) = m.count(all, &weights, &mut Counts::default());
        assert!((yes.log10() + 400.0).abs() < 1e-9);
        assert!((no.to_f64() - 1.0).abs() < 1e-15);
    }

    #[test]
    fn the_budget_stops_building() {
        let mut m = Manager::new(10, None);
        let vars: Vec<Bdd> = (0..20).map(|_| m.var()).collect();
        assert_eq!(m.stopped(), Some(Stop::TooLarge));
        let _ = vars;
        let cancel = Arc::new(AtomicBool::new(true));
        let mut m = Manager::new(1 << 20, Some(cancel));
        for _ in 0..CANCEL_CHECK {
            m.var();
        }
        assert_eq!(m.stopped(), Some(Stop::Cancelled));
    }
}
