//! Which variables may still be read after each statement.
//!
//! The engine merges worlds whose *live* variables are equal, and clears dead
//! ones, so that worlds differing only in values nobody reads again collapse
//! into one.

use crate::ir::*;

/// A fixed-size set of slot ids.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SlotSet {
    words: Vec<u64>,
}

impl SlotSet {
    pub fn with_capacity(n: usize) -> SlotSet {
        SlotSet {
            words: vec![0; n.div_ceil(64)],
        }
    }

    pub fn insert(&mut self, slot: SlotId) {
        let (w, b) = (slot as usize / 64, slot as usize % 64);
        if w >= self.words.len() {
            self.words.resize(w + 1, 0);
        }
        self.words[w] |= 1 << b;
    }

    pub fn remove(&mut self, slot: SlotId) {
        let (w, b) = (slot as usize / 64, slot as usize % 64);
        if w < self.words.len() {
            self.words[w] &= !(1 << b);
        }
    }

    pub fn contains(&self, slot: SlotId) -> bool {
        let (w, b) = (slot as usize / 64, slot as usize % 64);
        w < self.words.len() && self.words[w] & (1 << b) != 0
    }

    pub fn union_with(&mut self, other: &SlotSet) {
        if other.words.len() > self.words.len() {
            self.words.resize(other.words.len(), 0);
        }
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            *a |= b;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = SlotId> + '_ {
        self.words.iter().enumerate().flat_map(|(w, &bits)| {
            (0..64)
                .filter(move |b| bits & (1 << b) != 0)
                .map(move |b| (w * 64 + b) as SlotId)
        })
    }

    fn same(&self, other: &SlotSet) -> bool {
        let n = self.words.len().max(other.words.len());
        (0..n).all(|i| self.words.get(i).copied().unwrap_or(0) == other.words.get(i).copied().unwrap_or(0))
    }
}

/// Liveness for every statement of a program.
#[derive(Debug)]
pub struct Liveness {
    /// Slots that may be read after each statement, indexed by statement id.
    pub after: Vec<SlotSet>,
    /// For `Loop` statements: slots live at the start of each iteration.
    pub loop_head: Vec<SlotSet>,
}

pub fn analyze(program: &Program) -> Liveness {
    let n = program.stmt_count as usize;
    let mut liveness = Liveness {
        after: vec![SlotSet::default(); n],
        loop_head: vec![SlotSet::default(); n],
    };
    for func in &program.functions {
        let size = func.n_slots();
        let exit = SlotSet::with_capacity(size);
        let mut pass = Pass {
            live: &mut liveness,
            size,
        };
        let no_loop = LoopCtx {
            after: exit.clone(),
            head: exit.clone(),
        };
        pass.block(&func.body, exit, &no_loop);
    }
    liveness
}

struct LoopCtx {
    /// Live after the loop: where `break` goes.
    after: SlotSet,
    /// Live at the loop head: where `continue` goes.
    head: SlotSet,
}

struct Pass<'a> {
    live: &'a mut Liveness,
    size: usize,
}

impl Pass<'_> {
    fn block(&mut self, block: &Block, mut live: SlotSet, lc: &LoopCtx) -> SlotSet {
        for stmt in block.stmts.iter().rev() {
            self.live.after[stmt.id as usize] = live.clone();
            live = self.stmt(stmt, live, lc);
        }
        live
    }

    fn stmt(&mut self, stmt: &Stmt, out: SlotSet, lc: &LoopCtx) -> SlotSet {
        match &stmt.kind {
            StmtKind::Set { place, value } => {
                let mut live = out;
                define(place, &mut live);
                uses(value, &mut live);
                live
            }
            StmtKind::Draw { place, dist } => {
                let mut live = out;
                define(place, &mut live);
                uses(dist, &mut live);
                live
            }
            StmtKind::Take { place, bag } => {
                let mut live = out;
                define(place, &mut live);
                live.insert(bag.slot);
                place_uses(bag, &mut live);
                live
            }
            StmtKind::Call { dest, callee, args } => {
                let mut live = out;
                define(dest, &mut live);
                match callee {
                    Callee::Fn { capture_args, .. } => capture_args.iter().for_each(|&s| live.insert(s)),
                    Callee::Value(e) => uses(e, &mut live),
                }
                args.iter().for_each(|a| uses(a, &mut live));
                live
            }
            StmtKind::If { cond, then, otherwise } => {
                let mut live = self.block(then, out.clone(), lc);
                live.union_with(&self.block(otherwise, out, lc));
                uses(cond, &mut live);
                live
            }
            StmtKind::Chance {
                arms,
                otherwise,
                exhaustive,
            } => {
                let mut live = match otherwise {
                    Some(b) => self.block(b, out.clone(), lc),
                    None if !exhaustive => out.clone(),
                    None => SlotSet::with_capacity(self.size),
                };
                for (weight, body) in arms {
                    live.union_with(&self.block(body, out.clone(), lc));
                    uses(weight, &mut live);
                }
                live
            }
            StmtKind::Loop { body } => {
                let mut head = SlotSet::with_capacity(self.size);
                loop {
                    let inner = LoopCtx {
                        after: out.clone(),
                        head: head.clone(),
                    };
                    let mut entry = self.block(body, head.clone(), &inner);
                    entry.union_with(&head);
                    if entry.same(&head) {
                        break;
                    }
                    head = entry;
                }
                self.live.loop_head[stmt.id as usize] = head.clone();
                head
            }
            StmtKind::Break => lc.after.clone(),
            StmtKind::Continue => lc.head.clone(),
            StmtKind::Return(value) => {
                let mut live = SlotSet::with_capacity(self.size);
                uses(value, &mut live);
                live
            }
            StmtKind::Observe { value, from } => {
                let mut live = out;
                uses(value, &mut live);
                if let Some(d) = from {
                    uses(d, &mut live);
                }
                live
            }
            StmtKind::Report { value, key, .. } => {
                let mut live = out;
                uses(value, &mut live);
                if let Some(k) = key {
                    uses(k, &mut live);
                }
                live
            }
            StmtKind::Fail { .. } => SlotSet::with_capacity(self.size),
        }
    }
}

/// Writing to a whole variable kills it; writing into part of it doesn't.
fn define(place: &Place, live: &mut SlotSet) {
    if place.path.is_empty() {
        live.remove(place.slot);
    } else {
        live.insert(place.slot);
        place_uses(place, live);
    }
}

fn place_uses(place: &Place, live: &mut SlotSet) {
    for elem in &place.path {
        if let PathElem::Index(e) = elem {
            uses(e, live);
        }
    }
}

fn uses(expr: &Expr, live: &mut SlotSet) {
    match &expr.kind {
        ExprKind::Lit(_) => {}
        ExprKind::Slot(s) => live.insert(*s),
        ExprKind::Unary(_, e) | ExprKind::Field(e, _) => uses(e, live),
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
            uses(a, live);
            uses(b, live);
        }
        ExprKind::List(items) => items.iter().for_each(|e| uses(e, live)),
        ExprKind::Map(entries) => entries.iter().for_each(|(k, v)| {
            uses(k, live);
            uses(v, live);
        }),
        ExprKind::Record { fields, .. } => fields.iter().for_each(|(_, e)| uses(e, live)),
        ExprKind::With(base, fields) => {
            uses(base, live);
            fields.iter().for_each(|(_, e)| uses(e, live));
        }
        ExprKind::Builtin { args, named, .. } => {
            args.iter().for_each(|e| uses(e, live));
            named.iter().for_each(|(_, e)| uses(e, live));
        }
        ExprKind::Closure { capture_args, .. } | ExprKind::Simulate { capture_args, .. } => {
            capture_args.iter().for_each(|&s| live.insert(s))
        }
        ExprKind::Interp(parts) => parts.iter().for_each(|p| {
            if let InterpPart::Expr(e) = p {
                uses(e, live)
            }
        }),
    }
}
