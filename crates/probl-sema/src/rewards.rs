//! Rewards of loops (docs/semantics.md, section 10): variables that an
//! unbounded loop only adds to, and that nothing else in it reads, so that
//! they don't decide what the loop does. A loop whose other variables come
//! back to a state it was in can be solved as a Markov chain whose steps
//! carry how much each reward grows, like a counter of turns.

use crate::ir::{Block, Expr, ExprKind, PathElem, Program, SlotId, Stmt, StmtId, StmtKind};
use probl_syntax::ast::BinOp;
use rustc_hash::FxHashMap;

/// The unbounded loops that have rewards, by statement id, with them.
pub fn loop_rewards(program: &Program) -> FxHashMap<StmtId, Vec<SlotId>> {
    let mut rewards = FxHashMap::default();
    for func in &program.functions {
        each_loop(&func.body, &mut |s, body| {
            let found = of(body);
            if !found.is_empty() {
                rewards.insert(s.id, found);
            }
        });
    }
    rewards
}

fn each_loop(b: &Block, f: &mut impl FnMut(&Stmt, &Block)) {
    for s in &b.stmts {
        match &s.kind {
            StmtKind::Loop { body, bounded } => {
                if !bounded {
                    f(s, body);
                }
                each_loop(body, f);
            }
            StmtKind::If { then, otherwise, .. } => {
                each_loop(then, f);
                each_loop(otherwise, f);
            }
            StmtKind::Chance { arms, otherwise, .. } => {
                arms.iter().for_each(|(_, b)| each_loop(b, f));
                if let Some(b) = otherwise {
                    each_loop(b, f);
                }
            }
            StmtKind::Try { body, catches } => {
                each_loop(body, f);
                catches.iter().for_each(|c| each_loop(&c.body, f));
            }
            _ => {}
        }
    }
}

/// The slot a statement adds to, `r = r + e` or `r = r - e`, with what it
/// adds, if it's that, and `e` doesn't read `r`.
fn increment(s: &Stmt) -> Option<(SlotId, &Expr)> {
    let StmtKind::Set { place, value } = &s.kind else {
        return None;
    };
    if !place.path.is_empty() {
        return None;
    }
    let ExprKind::Binary(BinOp::Add | BinOp::Sub, a, e) = &value.kind else {
        return None;
    };
    if !matches!(a.kind, ExprKind::Slot(r) if r == place.slot) {
        return None;
    }
    let mut reads_itself = false;
    crate::draws::expr(e, &mut |s| reads_itself |= s == place.slot);
    (!reads_itself).then_some((place.slot, e))
}

/// A loop body's rewards: the slots it only adds to, and doesn't otherwise
/// read or write.
fn of(body: &Block) -> Vec<SlotId> {
    let mut added: Vec<SlotId> = Vec::new();
    let mut disqualified: Vec<SlotId> = Vec::new();
    each_stmt(body, &mut |s| {
        if let Some((r, e)) = increment(s) {
            if !added.contains(&r) {
                added.push(r);
            }
            crate::draws::expr(e, &mut |x| disqualified.push(x));
            return;
        }
        // A type check sees a reward's value as a step left it, which has
        // the type the whole reward has.
        if let StmtKind::Check { .. } = &s.kind {
            return;
        }
        // Any other write, or read, disqualifies the slot.
        writes(s, &mut |x| disqualified.push(x));
        own_reads(s, &mut |x| disqualified.push(x));
    });
    added.retain(|r| !disqualified.contains(r));
    added.sort_unstable();
    added
}

fn each_stmt(b: &Block, f: &mut impl FnMut(&Stmt)) {
    for s in &b.stmts {
        f(s);
        match &s.kind {
            StmtKind::If { then, otherwise, .. } => {
                each_stmt(then, f);
                each_stmt(otherwise, f);
            }
            StmtKind::Chance { arms, otherwise, .. } => {
                arms.iter().for_each(|(_, b)| each_stmt(b, f));
                if let Some(b) = otherwise {
                    each_stmt(b, f);
                }
            }
            StmtKind::Loop { body, .. } => each_stmt(body, f),
            StmtKind::Try { body, catches } => {
                each_stmt(body, f);
                catches.iter().for_each(|c| each_stmt(&c.body, f));
            }
            _ => {}
        }
    }
}

/// The slots a statement itself writes.
fn writes(s: &Stmt, f: &mut impl FnMut(SlotId)) {
    match &s.kind {
        StmtKind::Set { place, .. } | StmtKind::Update { place, .. } | StmtKind::Draw { place, .. } => f(place.slot),
        StmtKind::Take { place, bag } => {
            f(place.slot);
            f(bag.slot);
        }
        StmtKind::Call { dest, .. } => f(dest.slot),
        StmtKind::Check { slot, .. } => f(*slot),
        _ => {}
    }
}

/// The slots a statement itself reads, not counting the statements inside
/// it.
fn own_reads(s: &Stmt, f: &mut impl FnMut(SlotId)) {
    let place = |p: &crate::ir::Place, f: &mut dyn FnMut(SlotId)| {
        // Writing part of a variable reads the rest of it.
        if !p.path.is_empty() {
            f(p.slot);
        }
        for elem in &p.path {
            if let PathElem::Index(e) = elem {
                crate::draws::expr(e, &mut |x| f(x));
            }
        }
    };
    match &s.kind {
        StmtKind::Set { place: p, value } | StmtKind::Update { place: p, value, .. } => {
            place(p, f);
            crate::draws::expr(value, f);
        }
        StmtKind::Draw { place: p, dist } => {
            place(p, f);
            crate::draws::expr(dist, f);
        }
        StmtKind::Take { place: p, bag } => {
            place(p, f);
            f(bag.slot);
            place(bag, f);
        }
        StmtKind::Call { dest, callee, args } => {
            place(dest, f);
            match callee {
                crate::ir::Callee::Fn { capture_args, .. } => capture_args.iter().for_each(|&s| f(s)),
                crate::ir::Callee::Value(e, _) => crate::draws::expr(e, f),
            }
            args.iter().for_each(|a| crate::draws::expr(a, f));
        }
        StmtKind::If { cond, .. } => crate::draws::expr(cond, f),
        StmtKind::Chance { arms, .. } => arms.iter().for_each(|(w, _)| crate::draws::expr(w, f)),
        StmtKind::Return(e) => crate::draws::expr(e, f),
        StmtKind::Observe { value, from } => {
            crate::draws::expr(value, f);
            if let Some(d) = from {
                crate::draws::expr(d, f);
            }
        }
        StmtKind::Report { value, key, .. } => {
            crate::draws::expr(value, f);
            if let Some(k) = key {
                crate::draws::expr(k, f);
            }
        }
        StmtKind::Check { slot, .. } => f(*slot),
        StmtKind::Loop { .. } | StmtKind::Try { .. } | StmtKind::Break | StmtKind::Continue | StmtKind::Fail { .. } => {
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewards(src: &str) -> Vec<Vec<String>> {
        let (program, diags) = crate::compile(src);
        let program = program.unwrap_or_else(|| panic!("{diags:?}"));
        let main = program.main();
        let mut found: Vec<_> = loop_rewards(&program).into_iter().collect();
        found.sort_by_key(|(id, _)| *id);
        found
            .into_iter()
            .map(|(_, r)| r.iter().map(|&s| main.slots[s as usize].name.clone()).collect())
            .collect()
    }

    #[test]
    fn counters_that_nothing_reads_are_rewards() {
        assert_eq!(
            rewards("var n = 0\nvar t = 0.0\nwhile ~d6 != 6 {\n n += 1\n t = t - 0.5\n}\nreport n"),
            vec![vec!["n".to_string(), "t".to_string()]]
        );
        // Read by the condition, or another statement, or written otherwise:
        // not a reward.
        assert!(rewards("var n = 0\nwhile n < 10 { n += 1 }\nreport n").is_empty());
        assert!(rewards("var n = 0\nvar m = 0\nwhile ~d6 != 6 {\n n += 1\n m = n\n}\nreport m").is_empty());
        assert!(rewards("var n = 0\nwhile ~d6 != 6 {\n n += 1\n n = n * 2\n}\nreport n").is_empty());
        assert!(rewards("var n = 0\nwhile ~d6 != 6 {\n n = n + n\n}\nreport n").is_empty());
        // A typed counter, checked after each write.
        assert_eq!(
            rewards("var n: int = 0\nwhile ~d6 != 6 { n += 1 }\nreport n"),
            vec![vec!["n".to_string()]]
        );
        // Bounded loops aren't solved as chains.
        assert!(rewards("var n = 0\nfor i in 1..3 { n += 1 }\nreport n").is_empty());
    }
}
