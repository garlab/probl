//! Exact updates for conjugate priors, when sampling (docs/semantics.md,
//! section 14).
//!
//! A draw like `let a ~ beta(2, 50)` can be *delayed*: the variable keeps
//! its distribution, and an observation like `observe k from binomial(n, prob(a))`
//! updates that distribution exactly instead of weighting a drawn value.
//! Any other statement that reads the variable needs its value, so the
//! engine draws it first. This analysis finds, in each function, the
//! variables whose draws may be delayed, and for each statement, the ones it
//! must draw first.
//!
//! Whether a draw is delayed is decided when it runs: only when sampling,
//! and only a single beta, gamma or normal distribution.

use crate::Builtin;
use crate::draws;
use crate::ir::*;
use crate::liveness::SlotSet;
use probl_syntax::Span;

/// An `observe` that can update a delayed variable exactly.
#[derive(Clone, Copy, Debug)]
pub struct Update<'a> {
    /// The variable: the observed distribution's parameter.
    pub slot: SlotId,
    pub likelihood: Likelihood<'a>,
}

/// The forms of observation that are conjugate to a prior, with their parts
/// other than the variable.
#[derive(Clone, Copy, Debug)]
pub enum Likelihood<'a> {
    /// `observe value from binomial(trials, prob(x))`, for a beta prior.
    Binomial { value: &'a Expr, trials: &'a Expr },
    /// `observe value from bernoulli(prob(x))`, for a beta prior.
    Bernoulli { value: &'a Expr },
    /// `observe value from poisson(x)`, for a gamma prior.
    Poisson { value: &'a Expr },
    /// `observe value from normal(x, sd)`, for a normal prior.
    Normal { value: &'a Expr, sd: &'a Expr },
}

impl<'a> Update<'a> {
    /// The observation's parts besides the variable, in the order they're
    /// evaluated.
    pub fn others(&self) -> Vec<&'a Expr> {
        match self.likelihood {
            Likelihood::Binomial { value, trials } => vec![value, trials],
            Likelihood::Bernoulli { value } => vec![value],
            Likelihood::Poisson { value } => vec![value],
            Likelihood::Normal { value, sd } => vec![value, sd],
        }
    }
}

/// The exact update that `observe value from …` can make, if it has one of
/// the conjugate forms: the variable is the parameter, with `prob` for beta,
/// and appears nowhere else in the observation.
pub fn update<'a>(value: &'a Expr, from: Option<&'a Expr>) -> Option<Update<'a>> {
    let builtin = |e: &'a Expr| match &e.kind {
        ExprKind::Builtin { func, args, named } if named.is_empty() => Some((*func, args.as_slice())),
        _ => None,
    };
    let slot = |e: &Expr| match e.kind {
        ExprKind::Slot(s) => Some(s),
        _ => None,
    };
    // A beta draw is a float. The checked conversion makes its use as a
    // probability explicit, while the family guarantees the conversion.
    let probability_slot = |e: &'a Expr| match builtin(e)? {
        (Builtin::Prob, [x]) => slot(x),
        _ => None,
    };
    let mut distribution = builtin(from?)?;
    if let (Builtin::BooleanLaw, [inner]) = distribution {
        if let Some((Builtin::Bernoulli, args)) = builtin(inner) {
            distribution = (Builtin::Bernoulli, args);
        }
    }
    let (slot, likelihood) = match distribution {
        (Builtin::Binomial, [trials, x]) => (probability_slot(x)?, Likelihood::Binomial { value, trials }),
        (Builtin::Bernoulli | Builtin::BooleanLaw | Builtin::ScoreLaw, [x]) => {
            (probability_slot(x)?, Likelihood::Bernoulli { value })
        }
        (Builtin::Poisson, [x]) => (slot(x)?, Likelihood::Poisson { value }),
        (Builtin::Normal, [x, sd]) => (slot(x)?, Likelihood::Normal { value, sd }),
        _ => return None,
    };
    let update = Update { slot, likelihood };
    let mut elsewhere = false;
    for e in update.others() {
        draws::expr(e, &mut |s| elsewhere |= s == slot);
    }
    (!elsewhere).then_some(update)
}

/// What the engine needs to delay draws.
#[derive(Debug, Default)]
pub struct Conjugacy {
    /// The variables whose draws may be delayed.
    pub variables: Vec<Variable>,
    /// For each statement: if it's a draw that may be delayed, its variable
    /// (an index into `variables`).
    pub delays: Vec<Option<u32>>,
    /// For each statement: the variables that may be delayed and that it
    /// reads, which must be drawn before it runs. A conjugate observation
    /// doesn't read its variable, and a type check reads it only if the
    /// check could fail (the engine decides).
    pub draws_first: Vec<Vec<SlotId>>,
}

/// A variable whose draws may be delayed.
#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    pub function: FnId,
    pub slot: SlotId,
    pub name: String,
    /// Where it's declared.
    pub span: Span,
}

/// Find the variables whose draws may be delayed: in each function, those
/// drawn whole and observed through a conjugate form.
pub fn analyze(program: &Program) -> Conjugacy {
    let n = program.stmt_count as usize;
    let mut result = Conjugacy {
        variables: Vec::new(),
        delays: vec![None; n],
        draws_first: vec![Vec::new(); n],
    };
    for (f, func) in program.functions.iter().enumerate() {
        let mut observed = SlotSet::with_capacity(func.n_slots());
        let mut drawn = SlotSet::with_capacity(func.n_slots());
        each_stmt(&func.body, &mut |s| match &s.kind {
            StmtKind::Observe { value, from } => {
                if let Some(u) = update(value, from.as_ref()) {
                    observed.insert(u.slot);
                }
            }
            StmtKind::Draw { place, .. } if place.path.is_empty() => drawn.insert(place.slot),
            _ => {}
        });
        // The index in `variables` of each slot that may be delayed.
        let mut index = vec![None; func.n_slots()];
        for slot in observed.iter().filter(|&s| drawn.contains(s)) {
            index[slot as usize] = Some(result.variables.len() as u32);
            let info = &func.slots[slot as usize];
            result.variables.push(Variable {
                function: f as FnId,
                slot,
                name: info.name.clone(),
                span: info.span,
            });
        }
        if index.iter().all(Option::is_none) {
            continue;
        }
        each_stmt(&func.body, &mut |s| {
            if let StmtKind::Draw { place, .. } = &s.kind {
                if place.path.is_empty() {
                    result.delays[s.id as usize] = index[place.slot as usize];
                }
            }
            let mut reads: Vec<SlotId> = Vec::new();
            own_reads(s, &mut |slot| {
                if index[slot as usize].is_some() && !reads.contains(&slot) {
                    reads.push(slot);
                }
            });
            result.draws_first[s.id as usize] = reads;
        });
    }
    result
}

/// Call `f` with every statement in `b`, including the ones inside others.
fn each_stmt(b: &Block, f: &mut impl FnMut(&Stmt)) {
    for s in &b.stmts {
        f(s);
        match &s.kind {
            StmtKind::If { then, otherwise, .. } => {
                each_stmt(then, f);
                each_stmt(otherwise, f);
            }
            StmtKind::Chance { arms, otherwise, .. } => {
                for (_, body) in arms {
                    each_stmt(body, f);
                }
                if let Some(body) = otherwise {
                    each_stmt(body, f);
                }
            }
            StmtKind::Loop { body, .. } => each_stmt(body, f),
            _ => {}
        }
    }
}

/// Call `f` with every slot that `s` itself reads, not counting the
/// statements inside it, the variable a conjugate observation updates, or
/// the variable a type check checks. Assigning a whole variable doesn't
/// read it.
fn own_reads(s: &Stmt, f: &mut impl FnMut(SlotId)) {
    match &s.kind {
        StmtKind::Set { place, value } => {
            self::place(place, f);
            draws::value_reads(value, f);
        }
        StmtKind::Draw { place, dist } => {
            self::place(place, f);
            draws::value_reads(dist, f);
        }
        StmtKind::Take { place, bag } => {
            self::place(place, f);
            f(bag.slot);
            self::place(bag, f);
        }
        StmtKind::Call { dest, callee, args } => {
            self::place(dest, f);
            match callee {
                Callee::Fn { capture_args, .. } => capture_args.iter().for_each(|&s| f(s)),
                Callee::Value(e) => draws::value_reads(e, f),
            }
            args.iter().for_each(|a| draws::value_reads(a, f));
        }
        StmtKind::If { cond, .. } => draws::value_reads(cond, f),
        StmtKind::Chance { arms, .. } => arms.iter().for_each(|(weight, _)| draws::value_reads(weight, f)),
        StmtKind::Return(e) => draws::value_reads(e, f),
        StmtKind::Observe { value, from } => match update(value, from.as_ref()) {
            Some(u) => u.others().into_iter().for_each(|e| draws::value_reads(e, f)),
            None => {
                draws::value_reads(value, f);
                if let Some(d) = from {
                    draws::value_reads(d, f);
                }
            }
        },
        StmtKind::Report { value, key, .. } => {
            draws::value_reads(value, f);
            if let Some(k) = key {
                draws::value_reads(k, f);
            }
        }
        StmtKind::Loop { .. } | StmtKind::Break | StmtKind::Continue | StmtKind::Fail { .. } => {}
        StmtKind::Check { .. } => {}
    }
}

/// The slots a place reads: assigning part of a variable reads the rest of
/// it, and indices are read either way.
fn place(p: &Place, f: &mut impl FnMut(SlotId)) {
    if p.path.is_empty() {
        return;
    }
    f(p.slot);
    for elem in &p.path {
        if let PathElem::Index(e) = elem {
            draws::value_reads(e, f);
        }
    }
}
