//! What functions do besides computing a result, and the rule that no
//! `observe` may run after a `report` (docs/semantics.md, sections 6 and 7).

use crate::builtins::Builtin;
use crate::ir::*;
use probl_syntax::{Diagnostic, Span};
use std::collections::BTreeMap;

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

    // Effects reach callers through calls, `simulate` blocks (except their
    // observations, which are local to them), and calls of closures. The
    // closure called isn't known statically, so any lambda's effects count.
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
        lambda = Effects::default();
        for (f, e) in program.functions.iter().zip(&effects) {
            if f.kind == FnKind::Lambda {
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

/// What a function body does itself.
#[derive(Default)]
struct Direct {
    observes: bool,
    prints: bool,
    calls: Vec<FnId>,
    simulates: Vec<FnId>,
    calls_closures: bool,
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
                    Callee::Value(e) => {
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
            ExprKind::Lit(_) | ExprKind::Slot(_) | ExprKind::Closure { .. } => {}
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
                    Callee::Value(_) => (self.lambda_observes, None),
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
