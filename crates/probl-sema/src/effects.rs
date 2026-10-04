//! What functions do besides computing a result, and the rule that no
//! `observe` may run after a `report` (docs/semantics.md, sections 6 and 7).

use crate::builtins::Builtin;
use crate::ir::*;
use probl_syntax::{Diagnostic, Span};
use std::collections::{BTreeMap, BTreeSet};

/// Fill in `Function::effects` and check the observe-after-report rule.
pub fn analyze(program: &mut Program, src: &str) -> Vec<Diagnostic> {
    let n = program.functions.len();
    let direct: Vec<Direct> = program
        .functions
        .iter()
        .map(|f| {
            let mut d = Direct::default();
            d.block(&f.body);
            d
        })
        .collect();

    let referenced: BTreeSet<_> = direct.iter().flat_map(|d| d.references.iter().copied()).collect();
    let builtin_prints = direct.iter().any(|d| d.references_print);

    // Effects reach callers through calls, `simulate` blocks (except their
    // observations, which are local to them), and calls of closures. The
    // callee isn't known statically, so lambdas and referenced functions count.
    let mut effects: Vec<Effects> = direct
        .iter()
        .map(|d| Effects {
            observes: d.observes,
            prints: d.prints,
        })
        .collect();
    let mut lambda;
    loop {
        let mut changed = false;
        lambda = Effects {
            prints: builtin_prints,
            observes: false,
        };
        for (i, (f, e)) in program.functions.iter().zip(&effects).enumerate() {
            if f.kind == FnKind::Lambda || referenced.contains(&(i as FnId)) {
                lambda.prints |= e.prints;
                lambda.observes |= e.observes;
            }
        }
        for i in 0..n {
            let d = &direct[i];
            let mut e = effects[i];
            for &g in &d.calls {
                e.prints |= effects[g as usize].prints;
                e.observes |= effects[g as usize].observes;
            }
            for &c in &d.simulates {
                e.prints |= effects[c as usize].prints;
            }
            if d.calls_closures {
                e.prints |= lambda.prints;
                e.observes |= lambda.observes;
            }
            if e != effects[i] {
                effects[i] = e;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (f, e) in program.functions.iter_mut().zip(&effects) {
        f.effects = *e;
    }

    let mut checker = Checker {
        effects: &effects,
        lambda_observes: lambda.observes,
        functions: &program.functions,
        src,
        errors: BTreeMap::new(),
    };
    checker.block(&program.functions[MAIN as usize].body, None);
    checker.errors.into_values().collect()
}

/// Whether running `s` may print: itself, or through what it calls. Needs
/// the functions' effects, which `analyze` fills in.
pub(crate) fn indirect_prints(functions: &[Function]) -> bool {
    functions.iter().any(|f| {
        let mut refs = Direct::default();
        refs.block(&f.body);
        (f.kind == FnKind::Lambda && f.effects.prints)
            || refs.references_print
            || refs.references.iter().any(|&g| functions[g as usize].effects.prints)
    })
}

pub(crate) fn may_print(s: &Stmt, functions: &[Function], callable_prints: bool) -> bool {
    let mut d = Direct::default();
    d.stmt(s);
    d.prints
        || d.calls.iter().any(|&g| functions[g as usize].effects.prints)
        || d.simulates.iter().any(|&c| functions[c as usize].effects.prints)
        || (d.calls_closures && callable_prints)
}

/// For each statement: whether it's a `while` or `loop` that may be solved
/// as a Markov chain instead of unrolled (docs/semantics.md, section 10).
/// Its body mustn't report or print, which happen once per visit to a
/// state. Needs the functions' effects, which `analyze` fills in.
pub fn solvable_loops(program: &Program) -> Vec<bool> {
    fn visit(b: &Block, functions: &[Function], solvable: &mut [bool], callable_prints: bool) {
        for s in &b.stmts {
            match &s.kind {
                StmtKind::If { then, otherwise, .. } => {
                    visit(then, functions, solvable, callable_prints);
                    visit(otherwise, functions, solvable, callable_prints);
                }
                StmtKind::Chance { arms, otherwise, .. } => {
                    arms.iter()
                        .for_each(|(_, body)| visit(body, functions, solvable, callable_prints));
                    if let Some(body) = otherwise {
                        visit(body, functions, solvable, callable_prints);
                    }
                }
                StmtKind::Loop { body, bounded } => {
                    let mut d = Direct::default();
                    d.block(body);
                    solvable[s.id as usize] =
                        !bounded && !d.reports && !body.stmts.iter().any(|s| may_print(s, functions, callable_prints));
                    visit(body, functions, solvable, callable_prints);
                }
                _ => {}
            }
        }
    }
    let callable_prints = indirect_prints(&program.functions);
    let mut solvable = vec![false; program.stmt_count as usize];
    for f in &program.functions {
        visit(&f.body, &program.functions, &mut solvable, callable_prints);
    }
    solvable
}

/// What a function body does itself.
#[derive(Default)]
struct Direct {
    observes: bool,
    prints: bool,
    reports: bool,
    calls: Vec<FnId>,
    simulates: Vec<FnId>,
    calls_closures: bool,
    references: Vec<FnId>,
    references_print: bool,
}

impl Direct {
    fn block(&mut self, b: &Block) {
        for s in &b.stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Set { place, value } => {
                self.place(place);
                self.expr(value);
            }
            StmtKind::Draw { place, dist } => {
                self.place(place);
                self.expr(dist);
            }
            StmtKind::Take { place, bag } => {
                self.place(place);
                self.place(bag);
            }
            StmtKind::Call { dest, callee, args } => {
                self.place(dest);
                match callee {
                    Callee::Fn { func, .. } => self.calls.push(*func),
                    Callee::Value(e, _) => {
                        self.calls_closures = true;
                        self.expr(e);
                    }
                }
                args.iter().for_each(|a| self.expr(a));
            }
            StmtKind::If { cond, then, otherwise } => {
                self.expr(cond);
                self.block(then);
                self.block(otherwise);
            }
            StmtKind::Chance { arms, otherwise, .. } => {
                for (w, b) in arms {
                    self.expr(w);
                    self.block(b);
                }
                if let Some(b) = otherwise {
                    self.block(b);
                }
            }
            StmtKind::Loop { body, .. } => self.block(body),
            StmtKind::Return(e) => self.expr(e),
            StmtKind::Observe { value, from } => {
                self.observes = true;
                self.expr(value);
                if let Some(d) = from {
                    self.expr(d);
                }
            }
            StmtKind::Report { value, key, .. } => {
                self.reports = true;
                self.expr(value);
                if let Some(k) = key {
                    self.expr(k);
                }
            }
            StmtKind::Break | StmtKind::Continue | StmtKind::Fail { .. } | StmtKind::Check { .. } => {}
        }
    }

    fn place(&mut self, place: &Place) {
        for elem in &place.path {
            if let PathElem::Index(e) = elem {
                self.expr(e);
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Closure { func, .. } => self.references.push(*func),
            ExprKind::Lit(Lit::Builtin(Builtin::Print)) => self.references_print = true,
            ExprKind::Lit(_) | ExprKind::Slot(_) | ExprKind::Input(_) => {}
            ExprKind::Unary(_, x) | ExprKind::Field(x, _) => self.expr(x),
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::List(items) => items.iter().for_each(|x| self.expr(x)),
            ExprKind::Map(entries) => entries.iter().for_each(|(k, v)| {
                self.expr(k);
                self.expr(v);
            }),
            ExprKind::Record { fields, .. } => fields.iter().for_each(|(_, x)| self.expr(x)),
            ExprKind::With(base, fields) => {
                self.expr(base);
                fields.iter().for_each(|(_, x)| self.expr(x));
            }
            ExprKind::Builtin { func, args, named } => {
                match func {
                    Builtin::Print => self.prints = true,
                    Builtin::Map | Builtin::Filter | Builtin::Count | Builtin::Reduce => self.calls_closures = true,
                    Builtin::Minimum | Builtin::Maximum if args.len() >= 2 => self.calls_closures = true,
                    Builtin::Sort | Builtin::SortDesc if args.len() == 2 => self.calls_closures = true,
                    Builtin::Highest | Builtin::Lowest if args.len() == 3 => self.calls_closures = true,
                    _ => {}
                }
                args.iter().for_each(|x| self.expr(x));
                named.iter().for_each(|(_, x)| self.expr(x));
            }
            ExprKind::Simulate { func, .. } => self.simulates.push(*func),
            ExprKind::Interp(parts) => parts.iter().for_each(|p| {
                if let InterpPart::Expr(x) = p {
                    self.expr(x)
                }
            }),
        }
    }
}

/// Finds observations that can run after a report in the top-level code.
struct Checker<'a> {
    effects: &'a [Effects],
    lambda_observes: bool,
    functions: &'a [Function],
    src: &'a str,
    /// Keyed by position so each observation is reported once, in order.
    errors: BTreeMap<u32, Diagnostic>,
}

impl Checker<'_> {
    /// `reported` is a report that may already have run.
    fn block(&mut self, b: &Block, mut reported: Option<Span>) -> Option<Span> {
        for s in &b.stmts {
            reported = self.stmt(s, reported);
        }
        reported
    }

    fn stmt(&mut self, s: &Stmt, reported: Option<Span>) -> Option<Span> {
        match &s.kind {
            StmtKind::Report { .. } => reported.or(Some(s.span)),
            StmtKind::Observe { .. } => {
                if let Some(r) = reported {
                    self.error(s.span, r, None);
                }
                reported
            }
            StmtKind::Call { callee, .. } => {
                let (observes, name) = match callee {
                    Callee::Fn { func, .. } => (
                        self.effects[*func as usize].observes,
                        Some(self.functions[*func as usize].name.clone()),
                    ),
                    Callee::Value(..) => (self.lambda_observes, None),
                };
                if observes {
                    if let Some(r) = reported {
                        self.error(s.span, r, Some(name.unwrap_or_else(|| "this function".to_string())));
                    }
                }
                reported
            }
            StmtKind::If { then, otherwise, .. } => {
                let a = self.block(then, reported);
                let b = self.block(otherwise, reported);
                reported.or(a).or(b)
            }
            StmtKind::Chance { arms, otherwise, .. } => {
                let mut result = reported;
                for (_, body) in arms {
                    result = result.or(self.block(body, reported));
                }
                if let Some(body) = otherwise {
                    result = result.or(self.block(body, reported));
                }
                result
            }
            StmtKind::Loop { body, .. } => {
                let first = self.block(body, reported);
                if reported.is_none() && first.is_some() {
                    // The next iteration runs after this one's reports.
                    self.block(body, first);
                }
                reported.or(first)
            }
            _ => reported,
        }
    }

    fn error(&mut self, at: Span, report: Span, function: Option<String>) {
        let line = self.src[..(report.lo as usize).min(self.src.len())]
            .matches('\n')
            .count()
            + 1;
        let message = match &function {
            Some(name) => format!("`{name}` makes observations, and it can run after a `report`"),
            None => "this `observe` can run after a `report`".to_string(),
        };
        let d = Diagnostic::error(at, message)
            .with_label(format!("runs after the report on line {line}"))
            .with_note(
                "a report describes the evidence observed before it, so later observations would change its meaning",
            )
            .with_help("move the observations before the report, or the report after them");
        self.errors.entry(at.lo).or_insert(d);
    }
}
