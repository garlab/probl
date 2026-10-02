//! Moving draws to their first use (docs/benchmarks.md).
//!
//! `let x ~ D` moves down its block, to just before the first statement
//! that reads or writes `x`. Worlds then split on `x` only when something
//! needs it, and the facts the statements before combined have died, and
//! their worlds merged, by then. Written the natural way, with every
//! component's state drawn first, a 20-component reliability model follows
//! 256 worlds instead of 2²⁰.
//!
//! A draw moves only when that changes nothing but the number of worlds:
//!
//! - `D` is a distribution written out: dice, `bernoulli` of a probability,
//!   or `one_of` a list of values. It can't fail, reads no variable, and
//!   adds up to 1, so no weight goes unresolved, whenever it's drawn.
//! - It doesn't move past a statement that reads or writes `x`, prints
//!   (output is once per world), or can leave the block early (`break`,
//!   `continue`, `return`, or a failure).
//!
//! Everything else commutes with the draw: a statement that doesn't use `x`
//! does the same in every world the draw would have split, observations
//! multiply weights in either order, and reports add up the same weights.

use crate::effects::may_print;
use crate::ir::*;

/// Move every movable draw of the program down to its first use.
pub fn move_draws(program: &mut Program) {
    for i in 0..program.functions.len() {
        let body = std::mem::take(&mut program.functions[i].body);
        let body = block(body, &program.functions);
        program.functions[i].body = body;
    }
}

fn block(b: Block, functions: &[Function]) -> Block {
    let mut out: Vec<Stmt> = Vec::with_capacity(b.stmts.len());
    // Draws being carried down, in their order, with the slot each draws.
    // A draw's type check (`let x: bool ~ …`) travels with it.
    let mut carried: Vec<(Vec<Stmt>, SlotId)> = Vec::new();
    let mut just_carried = false;
    for mut s in b.stmts {
        if let (StmtKind::Check { slot, .. }, Some((draw, last))) = (&s.kind, carried.last_mut()) {
            if just_carried && slot == last {
                draw.push(s);
                continue;
            }
        }
        nested(&mut s, functions);
        // The draws that can't pass `s` land just before it, in order.
        let (land, pass): (Vec<_>, Vec<_>) = carried.into_iter().partition(|(_, slot)| stops(&s, *slot, functions));
        out.extend(land.into_iter().flat_map(|(d, _)| d));
        carried = pass;
        just_carried = match movable(&s) {
            Some(slot) => {
                carried.push((vec![s], slot));
                true
            }
            None => {
                out.push(s);
                false
            }
        };
    }
    out.extend(carried.into_iter().flat_map(|(d, _)| d));
    Block { stmts: out }
}

/// Move the draws in the blocks inside `s`.
fn nested(s: &mut Stmt, functions: &[Function]) {
    match &mut s.kind {
        StmtKind::If { then, otherwise, .. } => {
            *then = block(std::mem::take(then), functions);
            *otherwise = block(std::mem::take(otherwise), functions);
        }
        StmtKind::Chance { arms, otherwise, .. } => {
            for (_, b) in arms.iter_mut() {
                *b = block(std::mem::take(b), functions);
            }
            if let Some(b) = otherwise {
                *b = block(std::mem::take(b), functions);
            }
        }
        StmtKind::Loop { body, .. } => *body = block(std::mem::take(body), functions),
        _ => {}
    }
}

/// The slot a draw assigns, if it's a draw that may move.
fn movable(s: &Stmt) -> Option<SlotId> {
    let StmtKind::Draw { place, dist } = &s.kind else {
        return None;
    };
    (place.path.is_empty() && written_out(dist)).then_some(place.slot)
}

/// A distribution written out, which can't fail and adds up to 1.
fn written_out(e: &Expr) -> bool {
    let probability = |e: &Expr| matches!(e.kind, ExprKind::Lit(Lit::Prob(p)) if (0.0..=1.0).contains(&p));
    let value = |e: &Expr| {
        matches!(
            e.kind,
            ExprKind::Lit(
                Lit::Int(_)
                    | Lit::Float(_)
                    | Lit::FloatConstant(_)
                    | Lit::Prob(_)
                    | Lit::Str(_)
                    | Lit::Bool(_)
                    | Lit::Enum { .. }
            )
        )
    };
    match &e.kind {
        // Small enough never to reach a limit on outcomes.
        ExprKind::Lit(Lit::Dice { count, sides }) => (*count as u64) * (*sides as u64 - 1) < 10_000,
        ExprKind::Builtin { func, args, named } if named.is_empty() && args.len() == 1 => match func {
            crate::Builtin::Bernoulli => probability(&args[0]),
            crate::Builtin::OneOf => {
                matches!(&args[0].kind, ExprKind::List(items) if !items.is_empty() && items.len() <= 10_000 && items.iter().all(value))
            }
            _ => false,
        },
        _ => false,
    }
}

/// Whether a draw of `slot` must land before `s`.
fn stops(s: &Stmt, slot: SlotId, functions: &[Function]) -> bool {
    let mut uses = false;
    let mut leaves = false;
    visit(s, &mut |x| uses |= x == slot, &mut leaves);
    uses || leaves || may_print(s, functions)
}

/// Call `f` with every slot `s` reads or writes, anywhere inside it; set
/// `leaves` if it can leave its block early.
fn visit(s: &Stmt, f: &mut impl FnMut(SlotId), leaves: &mut bool) {
    match &s.kind {
        StmtKind::Set { place, value } => {
            self::place(place, f);
            expr(value, f);
        }
        StmtKind::Draw { place, dist } => {
            self::place(place, f);
            expr(dist, f);
        }
        StmtKind::Take { place, bag } => {
            self::place(place, f);
            self::place(bag, f);
        }
        StmtKind::Call { dest, callee, args } => {
            self::place(dest, f);
            match callee {
                Callee::Fn { capture_args, .. } => capture_args.iter().for_each(|&s| f(s)),
                Callee::Value(e) => expr(e, f),
            }
            args.iter().for_each(|a| expr(a, f));
        }
        StmtKind::If { cond, then, otherwise } => {
            expr(cond, f);
            then.stmts
                .iter()
                .chain(&otherwise.stmts)
                .for_each(|s| visit(s, f, leaves));
        }
        StmtKind::Chance { arms, otherwise, .. } => {
            for (w, b) in arms {
                expr(w, f);
                b.stmts.iter().for_each(|s| visit(s, f, leaves));
            }
            if let Some(b) = otherwise {
                b.stmts.iter().for_each(|s| visit(s, f, leaves));
            }
        }
        StmtKind::Loop { body, .. } => {
            // A `break` or `continue` inside a loop stays inside it.
            let mut inner = false;
            body.stmts.iter().for_each(|s| visit(s, f, &mut inner));
            *leaves |= returns(body);
        }
        StmtKind::Return(e) => {
            expr(e, f);
            *leaves = true;
        }
        StmtKind::Break | StmtKind::Continue | StmtKind::Fail { .. } => *leaves = true,
        StmtKind::Observe { value, from } => {
            expr(value, f);
            if let Some(d) = from {
                expr(d, f);
            }
        }
        StmtKind::Report { value, key, .. } => {
            expr(value, f);
            if let Some(k) = key {
                expr(k, f);
            }
        }
        StmtKind::Check { slot, .. } => f(*slot),
    }
}

/// Whether a block can return or fail, which leaves any loop it's in.
fn returns(b: &Block) -> bool {
    b.stmts.iter().any(|s| match &s.kind {
        StmtKind::Return(_) | StmtKind::Fail { .. } => true,
        StmtKind::If { then, otherwise, .. } => returns(then) || returns(otherwise),
        StmtKind::Chance { arms, otherwise, .. } => {
            arms.iter().any(|(_, b)| returns(b)) || otherwise.as_ref().is_some_and(returns)
        }
        StmtKind::Loop { body, .. } => returns(body),
        _ => false,
    })
}

fn place(p: &Place, f: &mut impl FnMut(SlotId)) {
    f(p.slot);
    for elem in &p.path {
        if let PathElem::Index(e) = elem {
            expr(e, f);
        }
    }
}

/// Call `f` with every slot `e` reads.
pub(crate) fn expr(e: &Expr, f: &mut impl FnMut(SlotId)) {
    expr_reads(e, f, true);
}

/// Value reads that force a delayed draw. Inspecting a slot with `typeof`
/// uses its outcome type without needing its value; liveness still reads it.
pub(crate) fn value_reads(e: &Expr, f: &mut impl FnMut(SlotId)) {
    expr_reads(e, f, false);
}

fn expr_reads(e: &Expr, f: &mut impl FnMut(SlotId), include_type_reads: bool) {
    let recurse = |e: &Expr, f: &mut _| expr_reads(e, f, include_type_reads);
    match &e.kind {
        ExprKind::Builtin {
            func: crate::Builtin::Typeof,
            args,
            ..
        } if !include_type_reads && matches!(args[0].kind, ExprKind::Slot(_)) => {}
        ExprKind::Lit(_) | ExprKind::Input(_) => {}
        ExprKind::Slot(s) => f(*s),
        ExprKind::Unary(_, x) | ExprKind::Field(x, _) => recurse(x, f),
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
            recurse(a, f);
            recurse(b, f);
        }
        ExprKind::List(items) => items.iter().for_each(|x| recurse(x, f)),
        ExprKind::Map(entries) => entries.iter().for_each(|(k, v)| {
            recurse(k, f);
            recurse(v, f);
        }),
        ExprKind::Record { fields, .. } => fields.iter().for_each(|(_, x)| recurse(x, f)),
        ExprKind::With(base, fields) => {
            recurse(base, f);
            fields.iter().for_each(|(_, x)| recurse(x, f));
        }
        ExprKind::Builtin { args, named, .. } => {
            args.iter().for_each(|x| recurse(x, f));
            named.iter().for_each(|(_, x)| recurse(x, f));
        }
        ExprKind::Closure { capture_args, .. } | ExprKind::Simulate { capture_args, .. } => {
            capture_args.iter().for_each(|&s| f(s))
        }
        ExprKind::Interp(parts) => parts.iter().for_each(|p| {
            if let InterpPart::Expr(x) = p {
                recurse(x, f)
            }
        }),
    }
}
