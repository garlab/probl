//! Name resolution and lowering from the AST to the IR.
//!
//! The pass resolves names to frame slots, checks the language's static rules
//! (functions only assign their own variables, `report` only at the top level,
//! …), and moves everything that can split worlds out of expressions into
//! statements of its own.

use crate::builtins::Builtin;
use crate::ir::*;
use probl_syntax::ast::{self, BinOp, UnOp};
use probl_syntax::{Diagnostic, Span};
use rustc_hash::FxHashMap;
use std::collections::{BTreeMap, BTreeSet};

/// Lower a parsed program. The IR is returned even when there are errors, but
/// it must not be run unless all diagnostics are warnings.
pub fn lower(program: &ast::Program, src: &str) -> (Program, Vec<Diagnostic>) {
    let mut lowerer = Lowerer::new(src);
    lowerer.program(program);
    lowerer.finish()
}

struct FnBuild {
    name: String,
    kind: FnKind,
    span: Span,
    n_params: u32,
    slots: Vec<SlotInfo>,
    parent: Option<FnId>,
    /// For lambdas and `simulate`: (slot here, slot in the parent).
    parent_captures: Vec<(SlotId, SlotId)>,
    /// Globals held by this function: main slot → slot here.
    global_slots: BTreeMap<SlotId, SlotId>,
    calls: BTreeSet<FnId>,
    children: Vec<FnId>,
    body: Block,
    captures: Vec<Capture>,
}

#[derive(Clone, Copy)]
struct Binding {
    slot: SlotId,
    mutable: bool,
    /// Copied in from outside the function, so it can't be assigned.
    captured: bool,
}

struct Ctx {
    func: FnId,
    scopes: Vec<FxHashMap<String, Binding>>,
    loops: u32,
}

enum Variant {
    One(u32, u32),
    Ambiguous,
}

struct Lowerer<'a> {
    src: &'a str,
    diags: Vec<Diagnostic>,
    funcs: Vec<FnBuild>,
    fn_by_name: FxHashMap<String, FnId>,
    records: Vec<RecordType>,
    record_by_name: FxHashMap<String, u32>,
    enums: Vec<EnumType>,
    enum_by_name: FxHashMap<String, u32>,
    variants: FxHashMap<String, Variant>,
    globals: FxHashMap<String, Binding>,
    reports: Vec<ReportSite>,
    settings: Settings,
    next_stmt: u32,
    ctx: Vec<Ctx>,
}

impl<'a> Lowerer<'a> {
    fn new(src: &'a str) -> Lowerer<'a> {
        Lowerer {
            src,
            diags: Vec::new(),
            funcs: Vec::new(),
            fn_by_name: FxHashMap::default(),
            records: Vec::new(),
            record_by_name: FxHashMap::default(),
            enums: Vec::new(),
            enum_by_name: FxHashMap::default(),
            variants: FxHashMap::default(),
            globals: FxHashMap::default(),
            reports: Vec::new(),
            settings: Settings::default(),
            next_stmt: 0,
            ctx: Vec::new(),
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn error(&mut self, span: Span, message: impl Into<String>) -> &mut Diagnostic {
        self.diags.push(Diagnostic::error(span, message));
        self.diags.last_mut().unwrap()
    }

    fn warning(&mut self, span: Span, message: impl Into<String>) -> &mut Diagnostic {
        self.diags.push(Diagnostic::warning(span, message));
        self.diags.last_mut().unwrap()
    }

    fn text(&self, span: Span) -> String {
        let raw = self.src.get(span.range()).unwrap_or("");
        raw.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn new_fn(&mut self, name: &str, kind: FnKind, span: Span, parent: Option<FnId>) -> FnId {
        self.funcs.push(FnBuild {
            name: name.to_string(),
            kind,
            span,
            n_params: 0,
            slots: Vec::new(),
            parent,
            parent_captures: Vec::new(),
            global_slots: BTreeMap::new(),
            calls: BTreeSet::new(),
            children: Vec::new(),
            body: Block::default(),
            captures: Vec::new(),
        });
        (self.funcs.len() - 1) as FnId
    }

    fn cur(&mut self) -> &mut Ctx {
        self.ctx.last_mut().unwrap()
    }

    fn cur_func(&self) -> FnId {
        self.ctx.last().unwrap().func
    }

    fn new_slot_in(&mut self, func: FnId, name: &str, span: Span) -> SlotId {
        let f = &mut self.funcs[func as usize];
        f.slots.push(SlotInfo {
            name: name.to_string(),
            span,
        });
        (f.slots.len() - 1) as SlotId
    }

    fn temp(&mut self, span: Span) -> SlotId {
        let func = self.cur_func();
        self.new_slot_in(func, "(temporary)", span)
    }

    fn stmt(&mut self, span: Span, kind: StmtKind) -> Stmt {
        let id = self.next_stmt;
        self.next_stmt += 1;
        Stmt { id, span, kind }
    }

    fn push_scope(&mut self) {
        self.cur().scopes.push(FxHashMap::default());
    }

    fn pop_scope(&mut self) {
        self.cur().scopes.pop();
    }

    fn at_top_level(&self) -> bool {
        self.ctx.len() == 1 && self.ctx[0].func == MAIN && self.ctx[0].scopes.len() == 1
    }

    /// Declare a variable in the innermost scope.
    fn declare(&mut self, name: &str, span: Span, mutable: bool) -> SlotId {
        let top = self.at_top_level();
        if top && self.globals.contains_key(name) {
            self.error(span, format!("`{name}` is already defined at the top level"))
                .help("top-level names must be unique; use `var` and assign to change a value");
        }
        let func = self.cur_func();
        let slot = self.new_slot_in(func, name, span);
        let binding = Binding {
            slot,
            mutable,
            captured: false,
        };
        self.cur().scopes.last_mut().unwrap().insert(name.to_string(), binding);
        if top {
            self.globals.insert(name.to_string(), binding);
        }
        slot
    }

    /// Resolve a variable name from the context at `depth`, capturing it into
    /// lambdas and functions as needed.
    fn lookup_at(&mut self, depth: usize, name: &str) -> Option<Binding> {
        for scope in self.ctx[depth].scopes.iter().rev() {
            if let Some(b) = scope.get(name) {
                return Some(*b);
            }
        }
        let func = self.ctx[depth].func;
        let kind = self.funcs[func as usize].kind;
        let found = match kind {
            FnKind::Main => return None,
            FnKind::Lambda | FnKind::Simulate if depth > 0 => {
                let outer = self.lookup_at(depth - 1, name)?;
                let span = self.funcs[func as usize].span;
                let slot = self.new_slot_in(func, name, span);
                self.funcs[func as usize].parent_captures.push((slot, outer.slot));
                slot
            }
            _ => {
                let global = *self.globals.get(name)?;
                let span = self.funcs[func as usize].span;
                let slot = self.new_slot_in(func, name, span);
                self.funcs[func as usize].global_slots.insert(global.slot, slot);
                slot
            }
        };
        let binding = Binding {
            slot: found,
            mutable: false,
            captured: true,
        };
        self.ctx[depth].scopes[0].insert(name.to_string(), binding);
        Some(binding)
    }

    fn lookup(&mut self, name: &str) -> Option<Binding> {
        let depth = self.ctx.len() - 1;
        self.lookup_at(depth, name)
    }

    fn variant(&mut self, name: &str, span: Span) -> Option<Lit> {
        match self.variants.get(name)? {
            Variant::One(ty, variant) => Some(Lit::Enum {
                ty: *ty,
                variant: *variant,
            }),
            Variant::Ambiguous => {
                self.error(span, format!("`{name}` is a variant of more than one enum"))
                    .help("write it with the enum's name, like `Market.Boom`");
                Some(Lit::Unit)
            }
        }
    }

    /// Names that could be meant at this point, nearest scopes first.
    fn visible_names(&self) -> Vec<String> {
        fn sorted<'s>(names: impl Iterator<Item = &'s String>) -> Vec<String> {
            let mut v: Vec<String> = names.cloned().collect();
            v.sort();
            v
        }
        let mut names: Vec<String> = Vec::new();
        for ctx in self.ctx.iter().rev() {
            for scope in ctx.scopes.iter().rev() {
                names.extend(sorted(scope.keys()));
            }
        }
        names.extend(sorted(self.globals.keys()));
        names.extend(sorted(self.fn_by_name.keys()));
        names.extend(sorted(self.variants.keys()));
        names.extend(
            Builtin::ALL
                .iter()
                .filter(|b| b.is_public())
                .map(|b| b.name().to_string()),
        );
        names
    }

    fn unknown_name(&mut self, name: &str, span: Span) {
        let suggestion = closest(name, &self.visible_names());
        let diag = self.error(span, format!("unknown name `{name}`"));
        if let Some(s) = suggestion {
            diag.help(format!("did you mean `{s}`?"));
        }
    }

    // ── Program ──────────────────────────────────────────────────────────

    fn program(&mut self, program: &ast::Program) {
        for pragma in &program.pragmas {
            self.pragma(pragma);
        }
        self.new_fn("the top level", FnKind::Main, Span::default(), None);

        // Declarations first, so they can be used before they're defined.
        let mut fn_decls = Vec::new();
        for item in &program.items {
            match item {
                ast::Item::Fn(decl) => {
                    let name = &decl.name.name;
                    if self.fn_by_name.contains_key(name) {
                        self.error(decl.name.span, format!("the function `{name}` is defined twice"));
                        continue;
                    }
                    if Builtin::from_name(name).is_some() {
                        self.warning(
                            decl.name.span,
                            format!("`{name}` hides the built-in function of the same name"),
                        );
                    }
                    let id = self.new_fn(name, FnKind::Named, decl.span, None);
                    self.funcs[id as usize].n_params = decl.params.len() as u32;
                    self.fn_by_name.insert(name.clone(), id);
                    fn_decls.push((id, decl));
                }
                ast::Item::Type(decl) => self.type_decl(decl),
                ast::Item::Enum(decl) => self.enum_decl(decl),
                ast::Item::Import(import) => {
                    self.error(import.span, "`import` isn't supported yet");
                }
                ast::Item::Stmt(_) => {}
            }
        }

        // The top level.
        self.ctx.push(Ctx {
            func: MAIN,
            scopes: vec![FxHashMap::default()],
            loops: 0,
        });
        let mut body = Vec::new();
        for item in &program.items {
            if let ast::Item::Stmt(stmt) = item {
                self.lower_stmt(stmt, &mut body);
            }
        }
        self.funcs[MAIN as usize].body = Block { stmts: body };
        self.ctx.pop();

        // Named functions, now that every top-level name is known.
        for (id, decl) in fn_decls {
            self.fn_body(id, decl);
        }
    }

    fn pragma(&mut self, pragma: &ast::Pragma) {
        let name = pragma.name.name.as_str();
        let arg = pragma.arg.as_ref();
        if name == "mode" {
            self.settings.mode_span = Some(pragma.span);
        }
        match name {
            "mode" => match arg.map(|a| &a.kind) {
                Some(ast::ExprKind::Name(mode)) => match mode.as_str() {
                    "exact" => self.settings.mode = Mode::Exact,
                    "auto" => self.settings.mode = Mode::Auto,
                    _ => self.bad_mode(pragma.span),
                },
                Some(ast::ExprKind::Call { callee, args }) => {
                    let ast::ExprKind::Name(mode) = &callee.kind else {
                        return self.bad_mode(pragma.span);
                    };
                    let mut named = FxHashMap::default();
                    for arg in args {
                        let Some(key) = &arg.name else {
                            self.error(arg.value.span, "mode settings are named, like `runs: 10_000`");
                            return;
                        };
                        match pragma_int(&arg.value) {
                            Some(v) => {
                                named.insert(key.name.as_str(), v);
                            }
                            None => {
                                self.error(arg.value.span, "expected a whole number");
                                return;
                            }
                        }
                    }
                    let runs = named.get("runs").copied().unwrap_or(10_000);
                    let seed = named.get("seed").copied().unwrap_or(0);
                    self.settings.mode = match mode.as_str() {
                        "sample" => Mode::Sample { runs, seed },
                        "particles" => Mode::Particles { runs, seed },
                        "beam" => Mode::Beam {
                            worlds: named.get("worlds").copied().unwrap_or(100_000),
                        },
                        _ => return self.bad_mode(pragma.span),
                    };
                }
                _ => self.bad_mode(pragma.span),
            },
            "epsilon" => match arg.and_then(pragma_float) {
                Some(e) if e > 0.0 && e < 1.0 => self.settings.epsilon = e,
                _ => {
                    self.error(
                        pragma.span,
                        "`@epsilon` needs a number between 0 and 1, like `@epsilon 1e-9`",
                    );
                }
            },
            "max_iterations" | "max_worlds" => match arg.and_then(pragma_int) {
                Some(v) if v > 0 => {
                    if name == "max_iterations" {
                        self.settings.max_iterations = v;
                    } else {
                        self.settings.max_worlds = v as usize;
                    }
                }
                _ => {
                    self.error(pragma.span, format!("`@{name}` needs a positive whole number"));
                }
            },
            _ => {
                self.error(pragma.name.span, format!("unknown pragma `@{name}`"))
                    .help("the pragmas are @mode, @epsilon, @max_iterations and @max_worlds");
            }
        }
    }

    fn bad_mode(&mut self, span: Span) {
        self.error(span, "unknown mode").help(
            "use `@mode exact`, `@mode auto`, `@mode sample(runs: 10_000, seed: 1)`, \
             `@mode particles(runs: 10_000, seed: 1)` or `@mode beam(worlds: 100_000)`",
        );
    }

    fn type_decl(&mut self, decl: &ast::TypeDecl) {
        let name = &decl.name.name;
        if self.record_by_name.contains_key(name) || self.enum_by_name.contains_key(name) {
            self.error(decl.name.span, format!("the type `{name}` is defined twice"));
            return;
        }
        let ast::TypeExpr::Record { fields, .. } = &decl.ty else {
            self.error(decl.span, "only record types can be declared for now")
                .help("write the fields in braces: `type Point = { x: int, y: int }`");
            return;
        };
        let mut names = Vec::new();
        for (field, _) in fields {
            if names.contains(&field.name) {
                self.error(field.span, format!("the field `{}` appears twice", field.name));
            }
            names.push(field.name.clone());
        }
        self.record_by_name.insert(name.clone(), self.records.len() as u32);
        self.records.push(RecordType {
            name: name.clone(),
            fields: names,
            span: decl.span,
        });
    }

    fn enum_decl(&mut self, decl: &ast::EnumDecl) {
        let name = &decl.name.name;
        if self.record_by_name.contains_key(name) || self.enum_by_name.contains_key(name) {
            self.error(decl.name.span, format!("the type `{name}` is defined twice"));
            return;
        }
        let ty = self.enums.len() as u32;
        let mut variants = Vec::new();
        for (i, variant) in decl.variants.iter().enumerate() {
            if variants.contains(&variant.name) {
                self.error(variant.span, format!("the variant `{}` appears twice", variant.name));
            }
            variants.push(variant.name.clone());
            let entry = match self.variants.get(&variant.name) {
                Some(_) => Variant::Ambiguous,
                None => Variant::One(ty, i as u32),
            };
            self.variants.insert(variant.name.clone(), entry);
        }
        self.enum_by_name.insert(name.clone(), ty);
        self.enums.push(EnumType {
            name: name.clone(),
            variants,
        });
    }

    fn fn_body(&mut self, id: FnId, decl: &ast::FnDecl) {
        self.ctx.push(Ctx {
            func: id,
            scopes: vec![FxHashMap::default()],
            loops: 0,
        });
        for param in &decl.params {
            if self.ctx[self.ctx.len() - 1].scopes[0].contains_key(&param.name.name) {
                self.error(
                    param.name.span,
                    format!("the parameter `{}` appears twice", param.name.name),
                );
            }
            self.declare(&param.name.name, param.name.span, false);
        }
        self.funcs[id as usize].n_params = decl.params.len() as u32;
        let mut stmts = Vec::new();
        self.push_scope();
        let value = self.block_value(&decl.body, &mut stmts);
        self.pop_scope();
        let ret = self.stmt(decl.body.span, StmtKind::Return(value));
        stmts.push(ret);
        self.funcs[id as usize].body = Block { stmts };
        self.ctx.pop();
    }

    // ── Statements ───────────────────────────────────────────────────────

    fn lower_stmt(&mut self, s: &ast::Stmt, out: &mut Vec<Stmt>) {
        match &s.kind {
            ast::StmtKind::Let {
                mutable,
                pattern,
                op,
                value,
                ty: _,
            } => self.let_stmt(*mutable, pattern, *op, value, s.span, out),
            ast::StmtKind::Assign { target, op, value } => self.assign(target, *op, value, s.span, out),
            ast::StmtKind::For { pattern, iter, body } => self.for_loop(pattern, iter, body, s.span, out),
            ast::StmtKind::While { cond, body } => {
                // loop { if cond { body } else { break } }
                self.cur().loops += 1;
                let mut inner = Vec::new();
                let c = self.expr(cond, &mut inner);
                let then = self.scoped_block(body);
                let brk = self.stmt(s.span, StmtKind::Break);
                let branch = self.stmt(
                    s.span,
                    StmtKind::If {
                        cond: c,
                        then,
                        otherwise: Block { stmts: vec![brk] },
                    },
                );
                inner.push(branch);
                self.cur().loops -= 1;
                let lp = self.stmt(
                    s.span,
                    StmtKind::Loop {
                        body: Block { stmts: inner },
                    },
                );
                out.push(lp);
            }
            ast::StmtKind::Repeat { count, body } => {
                let n = self.temp(s.span);
                let k = self.temp(s.span);
                let c = self.expr(count, out);
                let set_n = self.stmt(
                    s.span,
                    StmtKind::Set {
                        place: Place::slot(n),
                        value: builtin(Builtin::RepeatCount, vec![c], count.span),
                    },
                );
                let set_k = self.stmt(
                    s.span,
                    StmtKind::Set {
                        place: Place::slot(k),
                        value: lit(Lit::Int(0), s.span),
                    },
                );
                out.push(set_n);
                out.push(set_k);
                self.cur().loops += 1;
                let mut inner = Vec::new();
                let done = binary(BinOp::Ge, slot(k, s.span), slot(n, s.span), s.span);
                let brk = self.stmt(s.span, StmtKind::Break);
                let check = self.stmt(
                    s.span,
                    StmtKind::If {
                        cond: done,
                        then: Block { stmts: vec![brk] },
                        otherwise: Block::default(),
                    },
                );
                inner.push(check);
                let incr = self.stmt(
                    s.span,
                    StmtKind::Set {
                        place: Place::slot(k),
                        value: binary(BinOp::Add, slot(k, s.span), lit(Lit::Int(1), s.span), s.span),
                    },
                );
                inner.push(incr);
                let body = self.scoped_block(body);
                inner.extend(body.stmts);
                self.cur().loops -= 1;
                let lp = self.stmt(
                    s.span,
                    StmtKind::Loop {
                        body: Block { stmts: inner },
                    },
                );
                out.push(lp);
            }
            ast::StmtKind::Loop { body } => {
                self.cur().loops += 1;
                let body = self.scoped_block(body);
                self.cur().loops -= 1;
                let lp = self.stmt(s.span, StmtKind::Loop { body });
                out.push(lp);
            }
            ast::StmtKind::Break | ast::StmtKind::Continue => {
                let is_break = matches!(s.kind, ast::StmtKind::Break);
                if self.ctx.last().unwrap().loops == 0 {
                    let word = if is_break { "break" } else { "continue" };
                    self.error(s.span, format!("`{word}` outside of a loop"));
                    return;
                }
                let kind = if is_break { StmtKind::Break } else { StmtKind::Continue };
                let st = self.stmt(s.span, kind);
                out.push(st);
            }
            ast::StmtKind::Return(value) => {
                let kind = self.funcs[self.cur_func() as usize].kind;
                if kind == FnKind::Main {
                    self.error(s.span, "`return` outside of a function");
                    return;
                }
                let v = match value {
                    Some(v) => self.expr(v, out),
                    None => lit(Lit::Unit, s.span),
                };
                let st = self.stmt(s.span, StmtKind::Return(v));
                out.push(st);
            }
            ast::StmtKind::Observe { value, from } => {
                let v = self.expr(value, out);
                let d = from.as_ref().map(|d| self.expr(d, out));
                let st = self.stmt(s.span, StmtKind::Observe { value: v, from: d });
                out.push(st);
            }
            ast::StmtKind::Report { value, by, label } => self.report(value, by.as_ref(), label, s.span, out),
            ast::StmtKind::Expr(e) => self.expr_stmt(e, out),
        }
    }

    fn scoped_block(&mut self, block: &ast::Block) -> Block {
        self.push_scope();
        let mut stmts = Vec::new();
        for s in &block.stmts {
            self.lower_stmt(s, &mut stmts);
        }
        self.pop_scope();
        Block { stmts }
    }

    /// Lower the statements of a block and return the value of its last
    /// expression (or `()`). The caller manages the scope.
    fn block_value(&mut self, block: &ast::Block, out: &mut Vec<Stmt>) -> Expr {
        let Some((last, init)) = block.stmts.split_last() else {
            return lit(Lit::Unit, block.span);
        };
        for s in init {
            self.lower_stmt(s, out);
        }
        match &last.kind {
            ast::StmtKind::Expr(e) => self.expr(e, out),
            _ => {
                self.lower_stmt(last, out);
                lit(Lit::Unit, last.span)
            }
        }
    }

    fn let_stmt(
        &mut self,
        mutable: bool,
        pattern: &ast::Pattern,
        op: ast::BindOp,
        value: &ast::Expr,
        span: Span,
        out: &mut Vec<Stmt>,
    ) {
        // `let card ~ deck.take()`
        if op == ast::BindOp::Draw {
            if let Some(bag) = self.take_target(value, out) {
                let dest = match &pattern.kind {
                    ast::PatternKind::Name(name) => self.declare(name, pattern.span, mutable),
                    _ => {
                        let t = self.temp(span);
                        let st = self.stmt(
                            span,
                            StmtKind::Take {
                                place: Place::slot(t),
                                bag,
                            },
                        );
                        out.push(st);
                        return self.bind_pattern(pattern, slot(t, span), mutable, out);
                    }
                };
                let st = self.stmt(
                    span,
                    StmtKind::Take {
                        place: Place::slot(dest),
                        bag,
                    },
                );
                out.push(st);
                return;
            }
        }
        let v = self.expr(value, out);
        let kind = |place| match op {
            ast::BindOp::Assign => StmtKind::Set {
                place,
                value: v.clone(),
            },
            ast::BindOp::Draw => StmtKind::Draw { place, dist: v.clone() },
        };
        match &pattern.kind {
            ast::PatternKind::Name(name) => {
                let dest = self.declare(name, pattern.span, mutable);
                let st = self.stmt(span, kind(Place::slot(dest)));
                out.push(st);
            }
            _ => {
                let t = self.temp(span);
                let st = self.stmt(span, kind(Place::slot(t)));
                out.push(st);
                self.bind_pattern(pattern, slot(t, span), mutable, out);
            }
        }
    }

    /// Bind an irrefutable pattern (a name, `_`, or a list of those).
    fn bind_pattern(&mut self, pattern: &ast::Pattern, value: Expr, mutable: bool, out: &mut Vec<Stmt>) {
        match &pattern.kind {
            ast::PatternKind::Wildcard => {}
            ast::PatternKind::Name(name) => {
                let dest = self.declare(name, pattern.span, mutable);
                let st = self.stmt(
                    pattern.span,
                    StmtKind::Set {
                        place: Place::slot(dest),
                        value,
                    },
                );
                out.push(st);
            }
            ast::PatternKind::List(items) => {
                let check = builtin(
                    Builtin::IsListOfLen,
                    vec![value.clone(), lit(Lit::Int(items.len() as i64), pattern.span)],
                    pattern.span,
                );
                let fail = self.stmt(
                    pattern.span,
                    StmtKind::Fail {
                        message: format!("expected a list of {} items", items.len()),
                    },
                );
                let st = self.stmt(
                    pattern.span,
                    StmtKind::If {
                        cond: check,
                        then: Block::default(),
                        otherwise: Block { stmts: vec![fail] },
                    },
                );
                out.push(st);
                for (i, item) in items.iter().enumerate() {
                    let element = Expr {
                        kind: ExprKind::Index(Box::new(value.clone()), Box::new(lit(Lit::Int(i as i64), item.span))),
                        span: item.span,
                    };
                    self.bind_pattern(item, element, mutable, out);
                }
            }
            ast::PatternKind::Literal(_) | ast::PatternKind::Or(_) => {
                self.error(pattern.span, "this pattern might not match")
                    .help("use `match` to test values against patterns");
            }
        }
    }

    /// If `value` is `place.take()`, the bag to draw from.
    fn take_target(&mut self, value: &ast::Expr, out: &mut Vec<Stmt>) -> Option<Place> {
        let ast::ExprKind::Method { receiver, name, args } = &value.kind else {
            return None;
        };
        if name.name != "take" || self.fn_by_name.contains_key("take") {
            return None;
        }
        if !args.is_empty() {
            self.error(value.span, "`take()` doesn't take arguments");
        }
        self.place(receiver, out, "take a card from")
    }

    fn assign(&mut self, target: &ast::Expr, op: ast::AssignOp, value: &ast::Expr, span: Span, out: &mut Vec<Stmt>) {
        if op == ast::AssignOp::Draw {
            if let Some(bag) = self.take_target(value, out) {
                let Some(place) = self.place(target, out, "assign to") else {
                    return;
                };
                let st = self.stmt(span, StmtKind::Take { place, bag });
                out.push(st);
                return;
            }
        }
        let v = self.expr(value, out);
        let Some(place) = self.place(target, out, "assign to") else {
            return;
        };
        let kind = match op {
            ast::AssignOp::Set => StmtKind::Set { place, value: v },
            ast::AssignOp::Draw => StmtKind::Draw { place, dist: v },
            ast::AssignOp::Add | ast::AssignOp::Sub | ast::AssignOp::Mul | ast::AssignOp::Div => {
                let bin = match op {
                    ast::AssignOp::Add => BinOp::Add,
                    ast::AssignOp::Sub => BinOp::Sub,
                    ast::AssignOp::Mul => BinOp::Mul,
                    _ => BinOp::Div,
                };
                let current = place_read(&place, target.span);
                StmtKind::Set {
                    place,
                    value: binary(bin, current, v, span),
                }
            }
        };
        let st = self.stmt(span, kind);
        out.push(st);
    }

    /// Resolve an assignable place rooted at a mutable local variable.
    fn place(&mut self, e: &ast::Expr, out: &mut Vec<Stmt>, action: &str) -> Option<Place> {
        match &e.kind {
            ast::ExprKind::Name(name) => {
                let local = self
                    .ctx
                    .last()
                    .unwrap()
                    .scopes
                    .iter()
                    .rev()
                    .find_map(|s| s.get(name))
                    .copied();
                match local {
                    Some(b) if b.mutable => Some(Place::slot(b.slot)),
                    Some(b) if b.captured => {
                        self.error(e.span, format!("can't {action} `{name}` here"))
                            .note("functions, lambdas and `simulate` blocks can read outside variables but only change their own")
                            .help("return the new value instead");
                        None
                    }
                    Some(_) => {
                        self.error(e.span, format!("can't {action} `{name}`: it was declared with `let`"))
                            .help("declare it with `var` to be able to change it");
                        None
                    }
                    None => {
                        if self.lookup(name).is_some() {
                            self.error(e.span, format!("can't {action} `{name}` here"))
                                .note("functions, lambdas and `simulate` blocks can read outside variables but only change their own")
                                .help("return the new value instead");
                        } else {
                            self.unknown_name(name, e.span);
                        }
                        None
                    }
                }
            }
            ast::ExprKind::Field { expr, name } => {
                let mut place = self.place(expr, out, action)?;
                place.path.push(PathElem::Field(name.name.clone()));
                Some(place)
            }
            ast::ExprKind::Index { expr, index } => {
                let mut place = self.place(expr, out, action)?;
                let i = self.expr(index, out);
                place.path.push(PathElem::Index(i));
                Some(place)
            }
            _ => {
                self.error(e.span, format!("can't {action} this"))
                    .help("only variables, fields (`a.b`) and elements (`a[i]`) can be changed");
                None
            }
        }
    }

    fn for_loop(
        &mut self,
        pattern: &ast::Pattern,
        iter: &ast::Expr,
        body: &ast::Block,
        span: Span,
        out: &mut Vec<Stmt>,
    ) {
        let items = self.temp(iter.span);
        let index = self.temp(span);
        let collection = self.expr(iter, out);
        let set_items = self.stmt(
            iter.span,
            StmtKind::Set {
                place: Place::slot(items),
                value: builtin(Builtin::IterItems, vec![collection], iter.span),
            },
        );
        let set_index = self.stmt(
            span,
            StmtKind::Set {
                place: Place::slot(index),
                value: lit(Lit::Int(0), span),
            },
        );
        out.push(set_items);
        out.push(set_index);

        self.cur().loops += 1;
        self.push_scope();
        let mut inner = Vec::new();
        let len = builtin(Builtin::Len, vec![slot(items, span)], span);
        let done = binary(BinOp::Ge, slot(index, span), len, span);
        let brk = self.stmt(span, StmtKind::Break);
        let check = self.stmt(
            span,
            StmtKind::If {
                cond: done,
                then: Block { stmts: vec![brk] },
                otherwise: Block::default(),
            },
        );
        inner.push(check);
        let element = Expr {
            kind: ExprKind::Index(Box::new(slot(items, span)), Box::new(slot(index, span))),
            span: pattern.span,
        };
        self.bind_pattern(pattern, element, false, &mut inner);
        let incr = self.stmt(
            span,
            StmtKind::Set {
                place: Place::slot(index),
                value: binary(BinOp::Add, slot(index, span), lit(Lit::Int(1), span), span),
            },
        );
        inner.push(incr);
        let body = self.scoped_block(body);
        inner.extend(body.stmts);
        self.pop_scope();
        self.cur().loops -= 1;
        let lp = self.stmt(
            span,
            StmtKind::Loop {
                body: Block { stmts: inner },
            },
        );
        out.push(lp);
    }

    fn report(
        &mut self,
        value: &ast::Expr,
        by: Option<&ast::Expr>,
        label: &Option<(String, Span)>,
        span: Span,
        out: &mut Vec<Stmt>,
    ) {
        if self.ctx.len() > 1 || self.cur_func() != MAIN {
            self.error(span, "`report` is only allowed at the top level of the program")
                .help("return the value from the function and report it where it's called");
            return;
        }
        if by.is_none() && self.ctx[0].loops > 0 {
            self.error(span, "a `report` inside a loop needs `by`")
                .help("add a key that says which iteration each value belongs to, like `report x by month`");
            return;
        }
        let v = self.expr(value, out);
        let key = by.map(|k| self.expr(k, out));
        let site = self.reports.len() as u32;
        self.reports.push(ReportSite {
            label: match label {
                Some((text, _)) => text.clone(),
                None => self.text(value.span),
            },
            key_label: by.map(|k| self.text(k.span)),
            span,
        });
        let st = self.stmt(span, StmtKind::Report { site, value: v, key });
        out.push(st);
    }

    fn expr_stmt(&mut self, e: &ast::Expr, out: &mut Vec<Stmt>) {
        match &e.kind {
            ast::ExprKind::If { cond, then, otherwise } => {
                self.if_into(cond, then, otherwise.as_deref(), None, e.span, out)
            }
            ast::ExprKind::Chance { arms } => self.chance_into(arms, None, e.span, out),
            ast::ExprKind::Match { scrutinee, arms } => self.match_into(scrutinee, arms, None, e.span, out),
            ast::ExprKind::Block(block) => {
                let b = self.scoped_block(block);
                out.extend(b.stmts);
            }
            _ => {
                let v = self.expr(e, out);
                match &v.kind {
                    // Calls were hoisted into statements already.
                    ExprKind::Slot(_) | ExprKind::Lit(Lit::Unit) => {}
                    ExprKind::Builtin { .. } | ExprKind::Simulate { .. } => {
                        let t = self.temp(e.span);
                        let st = self.stmt(
                            e.span,
                            StmtKind::Set {
                                place: Place::slot(t),
                                value: v,
                            },
                        );
                        out.push(st);
                    }
                    _ => {
                        self.warning(e.span, "this value isn't used")
                            .help("use `report` to see a value, or `print` while debugging");
                    }
                }
            }
        }
    }

    // ── Branching ────────────────────────────────────────────────────────

    /// Lower an arm or branch body; with `dest`, its value is stored there.
    fn branch_body(&mut self, body: &ast::Stmt, dest: Option<SlotId>) -> Block {
        self.push_scope();
        let mut stmts = Vec::new();
        match (&body.kind, dest) {
            (ast::StmtKind::Expr(e), Some(d)) => self.expr_into(e, d, &mut stmts),
            (_, Some(d)) => {
                self.lower_stmt(body, &mut stmts);
                let st = self.stmt(
                    body.span,
                    StmtKind::Set {
                        place: Place::slot(d),
                        value: lit(Lit::Unit, body.span),
                    },
                );
                stmts.push(st);
            }
            (_, None) => self.lower_stmt(body, &mut stmts),
        }
        self.pop_scope();
        Block { stmts }
    }

    fn block_into(&mut self, block: &ast::Block, dest: Option<SlotId>) -> Block {
        self.push_scope();
        let mut stmts = Vec::new();
        match dest {
            Some(d) => {
                let v = self.block_value(block, &mut stmts);
                let st = self.stmt(
                    block.span,
                    StmtKind::Set {
                        place: Place::slot(d),
                        value: v,
                    },
                );
                stmts.push(st);
            }
            None => {
                for s in &block.stmts {
                    self.lower_stmt(s, &mut stmts);
                }
            }
        }
        self.pop_scope();
        Block { stmts }
    }

    fn if_into(
        &mut self,
        cond: &ast::Expr,
        then: &ast::Block,
        otherwise: Option<&ast::Expr>,
        dest: Option<SlotId>,
        span: Span,
        out: &mut Vec<Stmt>,
    ) {
        let c = self.expr(cond, out);
        let then = self.block_into(then, dest);
        let otherwise = match otherwise.map(|o| &o.kind) {
            Some(ast::ExprKind::If { cond, then, otherwise }) => {
                let mut stmts = Vec::new();
                self.push_scope();
                self.if_into(cond, then, otherwise.as_deref(), dest, span, &mut stmts);
                self.pop_scope();
                Block { stmts }
            }
            Some(ast::ExprKind::Block(block)) => self.block_into(block, dest),
            Some(_) => unreachable!("the parser only puts `if` or a block after `else`"),
            None => {
                if dest.is_some() {
                    self.error(span, "an `if` used as a value needs an `else`");
                }
                Block::default()
            }
        };
        let st = self.stmt(
            span,
            StmtKind::If {
                cond: c,
                then,
                otherwise,
            },
        );
        out.push(st);
    }

    fn chance_into(&mut self, arms: &[ast::ChanceArm], dest: Option<SlotId>, span: Span, out: &mut Vec<Stmt>) {
        let mut ir_arms = Vec::new();
        let mut otherwise = None;
        for (i, arm) in arms.iter().enumerate() {
            match &arm.weight {
                Some(w) => {
                    let weight = self.expr(w, out);
                    let body = self.branch_body(&arm.body, dest);
                    ir_arms.push((weight, body));
                }
                None => {
                    if i + 1 != arms.len() {
                        self.error(arm.span, "`else` must be the last arm of a `chance`");
                    }
                    otherwise = Some(self.branch_body(&arm.body, dest));
                }
            }
        }
        if ir_arms.is_empty() && otherwise.is_none() {
            self.error(span, "a `chance` needs at least one arm");
        }
        let st = self.stmt(
            span,
            StmtKind::Chance {
                arms: ir_arms,
                otherwise,
                exhaustive: dest.is_some(),
            },
        );
        out.push(st);
    }

    fn match_into(
        &mut self,
        scrutinee: &ast::Expr,
        arms: &[ast::MatchArm],
        dest: Option<SlotId>,
        span: Span,
        out: &mut Vec<Stmt>,
    ) {
        let subject = self.temp(scrutinee.span);
        let v = self.expr(scrutinee, out);
        let st = self.stmt(
            scrutinee.span,
            StmtKind::Set {
                place: Place::slot(subject),
                value: v,
            },
        );
        out.push(st);
        let message = format!(
            "no arm of this `match` matches the value of `{}`",
            self.text(scrutinee.span)
        );

        if arms.iter().all(|a| a.guard.is_none()) {
            // An `if … else if …` chain, built from the last arm backwards.
            let fail = self.stmt(span, StmtKind::Fail { message });
            let mut chain = Block { stmts: vec![fail] };
            for arm in arms.iter().rev() {
                self.push_scope();
                let mut then = Vec::new();
                let cond = self.pattern_test(&arm.pattern, &slot(subject, scrutinee.span), &mut then);
                let body = self.branch_body(&arm.body, dest);
                then.extend(body.stmts);
                self.pop_scope();
                let st = self.stmt(
                    arm.span,
                    StmtKind::If {
                        cond,
                        then: Block { stmts: then },
                        otherwise: chain,
                    },
                );
                chain = Block { stmts: vec![st] };
            }
            out.extend(chain.stmts);
            return;
        }

        // With guards, a flag records whether an arm has matched.
        let matched = self.temp(span);
        let init = self.stmt(
            span,
            StmtKind::Set {
                place: Place::slot(matched),
                value: lit(Lit::Prob(0.0), span),
            },
        );
        out.push(init);
        for arm in arms {
            self.push_scope();
            let mut then = Vec::new();
            let cond = self.pattern_test(&arm.pattern, &slot(subject, scrutinee.span), &mut then);
            let mark = self.stmt(
                arm.span,
                StmtKind::Set {
                    place: Place::slot(matched),
                    value: lit(Lit::Prob(1.0), arm.span),
                },
            );
            let mut body = vec![mark];
            body.extend(self.branch_body(&arm.body, dest).stmts);
            match &arm.guard {
                Some(guard) => {
                    let g = self.expr(guard, &mut then);
                    let st = self.stmt(
                        arm.span,
                        StmtKind::If {
                            cond: g,
                            then: Block { stmts: body },
                            otherwise: Block::default(),
                        },
                    );
                    then.push(st);
                }
                None => then.extend(body),
            }
            self.pop_scope();
            let not_matched = Expr {
                kind: ExprKind::Unary(UnOp::Not, Box::new(slot(matched, arm.span))),
                span: arm.span,
            };
            let st = self.stmt(
                arm.span,
                StmtKind::If {
                    cond: binary(BinOp::And, not_matched, cond, arm.span),
                    then: Block { stmts: then },
                    otherwise: Block::default(),
                },
            );
            out.push(st);
        }
        let fail = self.stmt(span, StmtKind::Fail { message });
        let unmatched = Expr {
            kind: ExprKind::Unary(UnOp::Not, Box::new(slot(matched, span))),
            span,
        };
        let st = self.stmt(
            span,
            StmtKind::If {
                cond: unmatched,
                then: Block { stmts: vec![fail] },
                otherwise: Block::default(),
            },
        );
        out.push(st);
    }

    /// The condition under which `value` matches `pattern`. Bindings are
    /// declared in the current scope and their assignments pushed to `binds`.
    fn pattern_test(&mut self, pattern: &ast::Pattern, value: &Expr, binds: &mut Vec<Stmt>) -> Expr {
        let span = pattern.span;
        match &pattern.kind {
            ast::PatternKind::Wildcard => lit(Lit::Prob(1.0), span),
            ast::PatternKind::Name(name) => {
                if let Some(variant) = self.variant(name, span) {
                    return binary(BinOp::Eq, value.clone(), lit(variant, span), span);
                }
                let dest = self.declare(name, span, false);
                let st = self.stmt(
                    span,
                    StmtKind::Set {
                        place: Place::slot(dest),
                        value: value.clone(),
                    },
                );
                binds.push(st);
                lit(Lit::Prob(1.0), span)
            }
            ast::PatternKind::Literal(e) => {
                let mut scratch = Vec::new();
                let l = self.expr(e, &mut scratch);
                binary(BinOp::Eq, value.clone(), l, span)
            }
            ast::PatternKind::List(items) => {
                let mut cond = builtin(
                    Builtin::IsListOfLen,
                    vec![value.clone(), lit(Lit::Int(items.len() as i64), span)],
                    span,
                );
                for (i, item) in items.iter().enumerate() {
                    let element = Expr {
                        kind: ExprKind::Index(Box::new(value.clone()), Box::new(lit(Lit::Int(i as i64), item.span))),
                        span: item.span,
                    };
                    let c = self.pattern_test(item, &element, binds);
                    cond = binary(BinOp::And, cond, c, span);
                }
                cond
            }
            ast::PatternKind::Or(alts) => {
                let before = binds.len();
                let mut cond: Option<Expr> = None;
                for alt in alts {
                    let c = self.pattern_test(alt, value, binds);
                    cond = Some(match cond {
                        None => c,
                        Some(prev) => binary(BinOp::Or, prev, c, span),
                    });
                }
                if binds.len() != before {
                    self.error(span, "patterns joined with `|` can't bind variables");
                    binds.truncate(before);
                }
                cond.unwrap()
            }
        }
    }

    // ── Expressions ──────────────────────────────────────────────────────

    /// Lower `e` and store its value in `dest`.
    fn expr_into(&mut self, e: &ast::Expr, dest: SlotId, out: &mut Vec<Stmt>) {
        match &e.kind {
            ast::ExprKind::If { cond, then, otherwise } => {
                self.if_into(cond, then, otherwise.as_deref(), Some(dest), e.span, out)
            }
            ast::ExprKind::Chance { arms } => self.chance_into(arms, Some(dest), e.span, out),
            ast::ExprKind::Match { scrutinee, arms } => self.match_into(scrutinee, arms, Some(dest), e.span, out),
            _ => {
                let v = self.expr(e, out);
                let st = self.stmt(
                    e.span,
                    StmtKind::Set {
                        place: Place::slot(dest),
                        value: v,
                    },
                );
                out.push(st);
            }
        }
    }

    fn expr(&mut self, e: &ast::Expr, out: &mut Vec<Stmt>) -> Expr {
        let span = e.span;
        let kind = match &e.kind {
            ast::ExprKind::Int(v) => ExprKind::Lit(Lit::Int(*v)),
            ast::ExprKind::Float(v) => ExprKind::Lit(Lit::Float(*v)),
            ast::ExprKind::Percent(v) => {
                if (0.0..=1.0).contains(v) {
                    ExprKind::Lit(Lit::Prob(*v))
                } else {
                    ExprKind::Lit(Lit::Float(*v))
                }
            }
            ast::ExprKind::Dice { count, sides } => ExprKind::Lit(Lit::Dice {
                count: *count,
                sides: *sides,
            }),
            ast::ExprKind::Bool(b) => ExprKind::Lit(Lit::Prob(if *b { 1.0 } else { 0.0 })),
            ast::ExprKind::Str(segments) => {
                let mut parts = Vec::new();
                for seg in segments {
                    match seg {
                        ast::StrSegment::Lit(text) => parts.push(InterpPart::Lit(text.clone())),
                        ast::StrSegment::Expr(inner) => parts.push(InterpPart::Expr(self.expr(inner, out))),
                    }
                }
                match parts.as_slice() {
                    [] => ExprKind::Lit(Lit::Str(String::new())),
                    [InterpPart::Lit(text)] => ExprKind::Lit(Lit::Str(text.clone())),
                    _ => ExprKind::Interp(parts),
                }
            }
            ast::ExprKind::Name(name) => return self.name(name, span),
            ast::ExprKind::List(items) => ExprKind::List(items.iter().map(|i| self.expr(i, out)).collect()),
            ast::ExprKind::Map(entries) => ExprKind::Map(
                entries
                    .iter()
                    .map(|(k, v)| {
                        let k = self.expr(k, out);
                        let v = self.expr(v, out);
                        (k, v)
                    })
                    .collect(),
            ),
            ast::ExprKind::Record { name, fields } => self.record(name.as_ref(), fields, span, out),
            ast::ExprKind::Unary { op, expr } => ExprKind::Unary(*op, Box::new(self.expr(expr, out))),
            ast::ExprKind::Binary { op, lhs, rhs } => return self.binary(*op, lhs, rhs, span, out),
            ast::ExprKind::Call { callee, args } => return self.call(callee, args, span, out),
            ast::ExprKind::Method { receiver, name, args } => return self.method(receiver, name, args, span, out),
            ast::ExprKind::Field { expr, name } => {
                if let ast::ExprKind::Name(base) = &expr.kind {
                    if let Some(&ty) = self.enum_by_name.get(base) {
                        if self.lookup(base).is_none() {
                            let enum_type = &self.enums[ty as usize];
                            return match enum_type.variants.iter().position(|v| *v == name.name) {
                                Some(i) => lit(Lit::Enum { ty, variant: i as u32 }, span),
                                None => {
                                    let enum_name = enum_type.name.clone();
                                    self.error(name.span, format!("`{enum_name}` has no variant `{}`", name.name));
                                    lit(Lit::Unit, span)
                                }
                            };
                        }
                    }
                }
                ExprKind::Field(Box::new(self.expr(expr, out)), name.name.clone())
            }
            ast::ExprKind::Index { expr, index } => {
                ExprKind::Index(Box::new(self.expr(expr, out)), Box::new(self.expr(index, out)))
            }
            ast::ExprKind::With { expr, fields } => {
                let base = self.expr(expr, out);
                let fields = self.fields(fields, out);
                ExprKind::With(Box::new(base), fields)
            }
            ast::ExprKind::Lambda { params, body } => return self.lambda(params, body, span),
            ast::ExprKind::Simulate(block) => return self.simulate(block, span),
            ast::ExprKind::If { .. } | ast::ExprKind::Chance { .. } | ast::ExprKind::Match { .. } => {
                let t = self.temp(span);
                self.expr_into(e, t, out);
                ExprKind::Slot(t)
            }
            ast::ExprKind::Block(block) => {
                self.push_scope();
                let v = self.block_value(block, out);
                self.pop_scope();
                return v;
            }
        };
        Expr { kind, span }
    }

    fn name(&mut self, name: &str, span: Span) -> Expr {
        if let Some(b) = self.lookup(name) {
            return slot(b.slot, span);
        }
        if let Some(variant) = self.variant(name, span) {
            return lit(variant, span);
        }
        if self.fn_by_name.contains_key(name) || Builtin::from_name(name).is_some() {
            self.error(span, format!("the function `{name}` can't be used as a value"))
                .help(format!("call it, or wrap it in a lambda: `x -> {name}(x)`"));
        } else if self.record_by_name.contains_key(name) || self.enum_by_name.contains_key(name) {
            self.error(span, format!("the type `{name}` can't be used as a value"));
        } else {
            self.unknown_name(name, span);
        }
        lit(Lit::Unit, span)
    }

    fn fields(&mut self, fields: &[ast::Field], out: &mut Vec<Stmt>) -> Vec<(String, Expr)> {
        let mut result: Vec<(String, Expr)> = Vec::new();
        for field in fields {
            if result.iter().any(|(n, _)| *n == field.name.name) {
                self.error(
                    field.name.span,
                    format!("the field `{}` appears twice", field.name.name),
                );
                continue;
            }
            let v = self.expr(&field.value, out);
            result.push((field.name.name.clone(), v));
        }
        result
    }

    fn record(
        &mut self,
        name: Option<&ast::Ident>,
        fields: &[ast::Field],
        span: Span,
        out: &mut Vec<Stmt>,
    ) -> ExprKind {
        let lowered = self.fields(fields, out);
        let Some(name) = name else {
            return ExprKind::Record {
                ty: None,
                fields: lowered,
            };
        };
        let Some(&ty) = self.record_by_name.get(&name.name) else {
            self.error(name.span, format!("unknown record type `{}`", name.name));
            return ExprKind::Lit(Lit::Unit);
        };
        let declared = self.records[ty as usize].fields.clone();
        for (field, _) in &lowered {
            if !declared.contains(field) {
                let at = fields.iter().find(|f| f.name.name == *field).unwrap().name.span;
                self.error(at, format!("`{}` has no field `{field}`", name.name));
            }
        }
        let missing: Vec<String> = declared
            .iter()
            .filter(|d| !lowered.iter().any(|(n, _)| n == *d))
            .map(|d| format!("`{d}`"))
            .collect();
        if !missing.is_empty() {
            self.error(span, format!("missing {} in `{}`", missing.join(", "), name.name));
        }
        ExprKind::Record {
            ty: Some(ty),
            fields: lowered,
        }
    }

    fn binary(&mut self, op: BinOp, lhs: &ast::Expr, rhs: &ast::Expr, span: Span, out: &mut Vec<Stmt>) -> Expr {
        let l = self.expr(lhs, out);
        if matches!(op, BinOp::And | BinOp::Or) {
            // The right side must only run when it's needed. If lowering it
            // produced statements, they go into a branch.
            let mut rhs_stmts = Vec::new();
            let r = self.expr(rhs, &mut rhs_stmts);
            if rhs_stmts.is_empty() {
                return binary(op, l, r, span);
            }
            let t = self.temp(span);
            let set = self.stmt(
                span,
                StmtKind::Set {
                    place: Place::slot(t),
                    value: l,
                },
            );
            out.push(set);
            let test = if op == BinOp::And {
                Builtin::IsFalse
            } else {
                Builtin::IsTrue
            };
            let combine = self.stmt(
                span,
                StmtKind::Set {
                    place: Place::slot(t),
                    value: binary(op, slot(t, span), r, span),
                },
            );
            rhs_stmts.push(combine);
            let st = self.stmt(
                span,
                StmtKind::If {
                    cond: builtin(test, vec![slot(t, span)], span),
                    then: Block::default(),
                    otherwise: Block { stmts: rhs_stmts },
                },
            );
            out.push(st);
            return slot(t, span);
        }
        let r = self.expr(rhs, out);
        if op == BinOp::To {
            return builtin(Builtin::To, vec![l, r], span);
        }
        binary(op, l, r, span)
    }

    fn call(&mut self, callee: &ast::Expr, args: &[ast::Arg], span: Span, out: &mut Vec<Stmt>) -> Expr {
        if let ast::ExprKind::Name(name) = &callee.kind {
            if self.lookup(name).is_none() {
                let ident = ast::Ident {
                    name: name.clone(),
                    span: callee.span,
                };
                return self.call_named(&ident, None, args, span, out);
            }
        }
        // Calling a closure value.
        let f = self.expr(callee, out);
        let args = self.positional_args(args, out);
        self.hoist_call(Callee::Value(f), args, span, out)
    }

    fn method(
        &mut self,
        receiver: &ast::Expr,
        name: &ast::Ident,
        args: &[ast::Arg],
        span: Span,
        out: &mut Vec<Stmt>,
    ) -> Expr {
        if let Some(b) = Builtin::from_name(&name.name) {
            if !self.fn_by_name.contains_key(&name.name) {
                if b == Builtin::Take {
                    self.error(span, "`take()` draws a card, so it has to be used with `~`")
                        .help("write `let card ~ deck.take()`");
                    return lit(Lit::Unit, span);
                }
                if b.is_mutating() {
                    return self.mutating_method(b, receiver, name, args, span, out);
                }
            }
        }
        self.call_named(name, Some(receiver), args, span, out)
    }

    fn mutating_method(
        &mut self,
        b: Builtin,
        receiver: &ast::Expr,
        name: &ast::Ident,
        args: &[ast::Arg],
        span: Span,
        out: &mut Vec<Stmt>,
    ) -> Expr {
        let Some(place) = self.place(receiver, out, &format!("`{}` to", name.name)) else {
            return lit(Lit::Unit, span);
        };
        let mut all = vec![place_read(&place, receiver.span)];
        all.extend(self.positional_args(args, out));
        self.check_arity(b, all.len(), span);
        let current = place_read(&place, receiver.span);
        if b == Builtin::Pop {
            let t = self.temp(span);
            let take_last = self.stmt(
                span,
                StmtKind::Set {
                    place: Place::slot(t),
                    value: builtin(Builtin::Last, vec![current.clone()], span),
                },
            );
            let shorten = self.stmt(
                span,
                StmtKind::Set {
                    place,
                    value: builtin(Builtin::DropLast, vec![current], span),
                },
            );
            out.push(take_last);
            out.push(shorten);
            return slot(t, span);
        }
        let st = self.stmt(
            span,
            StmtKind::Set {
                place,
                value: builtin(b, all, span),
            },
        );
        out.push(st);
        lit(Lit::Unit, span)
    }

    fn call_named(
        &mut self,
        name: &ast::Ident,
        receiver: Option<&ast::Expr>,
        args: &[ast::Arg],
        span: Span,
        out: &mut Vec<Stmt>,
    ) -> Expr {
        let mut values = Vec::new();
        if let Some(r) = receiver {
            values.push(self.expr(r, out));
        }
        if let Some(&func) = self.fn_by_name.get(&name.name) {
            values.extend(self.positional_args(args, out));
            let caller = self.cur_func();
            self.funcs[caller as usize].calls.insert(func);
            let n_params = self.fn_param_count(func);
            if values.len() != n_params {
                self.error(
                    span,
                    format!(
                        "`{}` takes {} argument{}, but {} {} given",
                        name.name,
                        n_params,
                        plural(n_params),
                        values.len(),
                        if values.len() == 1 { "was" } else { "were" }
                    ),
                );
            }
            return self.hoist_call(
                Callee::Fn {
                    func,
                    capture_args: Vec::new(),
                },
                values,
                span,
                out,
            );
        }
        if let Some(b) = Builtin::from_name(&name.name) {
            let mut named = Vec::new();
            for arg in args {
                let v = self.expr(&arg.value, out);
                match &arg.name {
                    Some(n) => named.push((n.name.clone(), v)),
                    None => values.push(v),
                }
            }
            if !named.is_empty() {
                self.error(span, format!("`{}` doesn't take named arguments", name.name));
            }
            self.check_arity(b, values.len(), span);
            return Expr {
                kind: ExprKind::Builtin {
                    func: b,
                    args: values,
                    named,
                },
                span,
            };
        }
        if self.record_by_name.contains_key(&name.name) {
            self.error(name.span, format!("`{}` is a record type", name.name))
                .help(format!("create one with braces: `{} {{ field: value }}`", name.name));
        } else {
            let suggestion = closest(&name.name, &self.visible_names());
            let d = self.error(name.span, format!("unknown function `{}`", name.name));
            if let Some(s) = suggestion {
                d.help(format!("did you mean `{s}`?"));
            }
        }
        lit(Lit::Unit, span)
    }

    fn fn_param_count(&self, func: FnId) -> usize {
        self.funcs[func as usize].n_params as usize
    }

    fn check_arity(&mut self, b: Builtin, n: usize, span: Span) {
        let (min, max) = b.arity();
        if n < min || n > max {
            let expected = if min == max {
                format!("{min} argument{}", plural(min))
            } else if max == usize::MAX {
                format!("at least {min} argument{}", plural(min))
            } else {
                format!("{min} to {max} arguments")
            };
            self.error(
                span,
                format!(
                    "`{}` takes {expected}, but {n} {} given",
                    b.name(),
                    if n == 1 { "was" } else { "were" }
                ),
            );
        }
    }

    fn positional_args(&mut self, args: &[ast::Arg], out: &mut Vec<Stmt>) -> Vec<Expr> {
        args.iter()
            .map(|arg| {
                if let Some(n) = &arg.name {
                    self.error(n.span, "only built-in functions take named arguments");
                }
                self.expr(&arg.value, out)
            })
            .collect()
    }

    fn hoist_call(&mut self, callee: Callee, args: Vec<Expr>, span: Span, out: &mut Vec<Stmt>) -> Expr {
        let t = self.temp(span);
        let st = self.stmt(
            span,
            StmtKind::Call {
                dest: Place::slot(t),
                callee,
                args,
            },
        );
        out.push(st);
        slot(t, span)
    }

    fn lambda(&mut self, params: &[ast::Ident], body: &ast::Expr, span: Span) -> Expr {
        let parent = self.cur_func();
        let id = self.new_fn("a lambda", FnKind::Lambda, span, Some(parent));
        self.funcs[parent as usize].children.push(id);
        self.ctx.push(Ctx {
            func: id,
            scopes: vec![FxHashMap::default()],
            loops: 0,
        });
        for p in params {
            self.declare(&p.name, p.span, false);
        }
        self.funcs[id as usize].n_params = params.len() as u32;
        let mut stmts = Vec::new();
        self.push_scope();
        let v = self.expr(body, &mut stmts);
        self.pop_scope();
        let ret = self.stmt(body.span, StmtKind::Return(v));
        stmts.push(ret);
        self.funcs[id as usize].body = Block { stmts };
        self.ctx.pop();
        Expr {
            kind: ExprKind::Closure {
                func: id,
                capture_args: Vec::new(),
            },
            span,
        }
    }

    fn simulate(&mut self, block: &ast::Block, span: Span) -> Expr {
        let parent = self.cur_func();
        let id = self.new_fn("a simulate block", FnKind::Simulate, span, Some(parent));
        self.funcs[parent as usize].children.push(id);
        self.ctx.push(Ctx {
            func: id,
            scopes: vec![FxHashMap::default()],
            loops: 0,
        });
        let mut stmts = Vec::new();
        self.push_scope();
        let v = self.block_value(block, &mut stmts);
        self.pop_scope();
        let ret = self.stmt(block.span, StmtKind::Return(v));
        stmts.push(ret);
        self.funcs[id as usize].body = Block { stmts };
        self.ctx.pop();
        Expr {
            kind: ExprKind::Simulate {
                func: id,
                capture_args: Vec::new(),
            },
            span,
        }
    }

    // ── Finishing ────────────────────────────────────────────────────────

    fn finish(mut self) -> (Program, Vec<Diagnostic>) {
        let n = self.funcs.len();

        // Which globals each function needs, including for its callees and
        // the lambdas it creates.
        let mut needed: Vec<BTreeSet<SlotId>> = self
            .funcs
            .iter()
            .map(|f| f.global_slots.keys().copied().collect())
            .collect();
        needed[MAIN as usize].clear();
        loop {
            let mut changed = false;
            for f in 1..n {
                let mut acc = needed[f].clone();
                for &g in &self.funcs[f].calls {
                    acc.extend(needed[g as usize].iter().copied());
                }
                for &c in &self.funcs[f].children {
                    acc.extend(needed[c as usize].iter().copied());
                }
                if acc != needed[f] {
                    needed[f] = acc;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        // Give every function a slot for each global it needs. Parents are
        // created before their lambdas, so they're handled first.
        for (f, want) in needed.iter().enumerate().skip(1) {
            for &g in want {
                if self.funcs[f].global_slots.contains_key(&g) {
                    continue;
                }
                let name = self.funcs[MAIN as usize].slots[g as usize].name.clone();
                let span = self.funcs[f].span;
                let slot = self.new_slot_in(f as FnId, &name, span);
                self.funcs[f].global_slots.insert(g, slot);
                if let Some(parent) = self.funcs[f].parent {
                    let source = if parent == MAIN {
                        g
                    } else {
                        self.funcs[parent as usize].global_slots[&g]
                    };
                    self.funcs[f].parent_captures.push((slot, source));
                }
            }
        }

        for f in &mut self.funcs {
            f.captures = match f.kind {
                FnKind::Main => Vec::new(),
                FnKind::Named => f
                    .global_slots
                    .iter()
                    .map(|(&g, &slot)| Capture {
                        slot,
                        source: CaptureSource::Global(g),
                    })
                    .collect(),
                FnKind::Lambda | FnKind::Simulate => f
                    .parent_captures
                    .iter()
                    .map(|&(slot, source)| Capture {
                        slot,
                        source: CaptureSource::Parent(source),
                    })
                    .collect(),
            };
        }

        // Fill in the arguments that supply captures at calls and closures.
        let captures: Vec<Vec<Capture>> = self.funcs.iter().map(|f| f.captures.clone()).collect();
        let global_slots: Vec<BTreeMap<SlotId, SlotId>> = self.funcs.iter().map(|f| f.global_slots.clone()).collect();
        for (f, func) in self.funcs.iter_mut().enumerate() {
            let slot_of_global = |g: SlotId| -> SlotId { if f == MAIN as usize { g } else { global_slots[f][&g] } };
            let mut fill = |site: &mut CallSite| {
                let (CallSite::Call(callee, args) | CallSite::Closure(callee, args)) = site;
                **args = captures[*callee as usize]
                    .iter()
                    .map(|c| match c.source {
                        CaptureSource::Global(g) => slot_of_global(g),
                        CaptureSource::Parent(s) => s,
                    })
                    .collect();
            };
            visit_block(&mut func.body, &mut fill);
        }

        let functions = self
            .funcs
            .into_iter()
            .map(|f| Function {
                name: f.name,
                kind: f.kind,
                span: f.span,
                n_params: f.n_params,
                captures: f.captures,
                slots: f.slots,
                body: f.body,
            })
            .collect();
        let program = Program {
            functions,
            reports: self.reports,
            records: self.records,
            enums: self.enums,
            settings: self.settings,
            stmt_count: self.next_stmt,
        };
        (program, self.diags)
    }
}

enum CallSite<'a> {
    Call(FnId, &'a mut Vec<SlotId>),
    Closure(FnId, &'a mut Vec<SlotId>),
}

fn visit_block(block: &mut Block, f: &mut impl FnMut(&mut CallSite)) {
    for stmt in &mut block.stmts {
        visit_stmt(stmt, f);
    }
}

fn visit_stmt(stmt: &mut Stmt, f: &mut impl FnMut(&mut CallSite)) {
    match &mut stmt.kind {
        StmtKind::Set { place, value } => {
            visit_place(place, f);
            visit_expr(value, f);
        }
        StmtKind::Draw { place, dist } => {
            visit_place(place, f);
            visit_expr(dist, f);
        }
        StmtKind::Take { place, bag } => {
            visit_place(place, f);
            visit_place(bag, f);
        }
        StmtKind::Call { dest, callee, args } => {
            visit_place(dest, f);
            match callee {
                Callee::Fn { func, capture_args } => f(&mut CallSite::Call(*func, capture_args)),
                Callee::Value(e) => visit_expr(e, f),
            }
            for a in args {
                visit_expr(a, f);
            }
        }
        StmtKind::If { cond, then, otherwise } => {
            visit_expr(cond, f);
            visit_block(then, f);
            visit_block(otherwise, f);
        }
        StmtKind::Chance { arms, otherwise, .. } => {
            for (w, b) in arms {
                visit_expr(w, f);
                visit_block(b, f);
            }
            if let Some(b) = otherwise {
                visit_block(b, f);
            }
        }
        StmtKind::Loop { body } => visit_block(body, f),
        StmtKind::Return(e) => visit_expr(e, f),
        StmtKind::Observe { value, from } => {
            visit_expr(value, f);
            if let Some(d) = from {
                visit_expr(d, f);
            }
        }
        StmtKind::Report { value, key, .. } => {
            visit_expr(value, f);
            if let Some(k) = key {
                visit_expr(k, f);
            }
        }
        StmtKind::Break | StmtKind::Continue | StmtKind::Fail { .. } => {}
    }
}

fn visit_place(place: &mut Place, f: &mut impl FnMut(&mut CallSite)) {
    for elem in &mut place.path {
        if let PathElem::Index(e) = elem {
            visit_expr(e, f);
        }
    }
}

fn visit_expr(expr: &mut Expr, f: &mut impl FnMut(&mut CallSite)) {
    match &mut expr.kind {
        ExprKind::Lit(_) | ExprKind::Slot(_) => {}
        ExprKind::Unary(_, e) | ExprKind::Field(e, _) => visit_expr(e, f),
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
            visit_expr(a, f);
            visit_expr(b, f);
        }
        ExprKind::List(items) => items.iter_mut().for_each(|e| visit_expr(e, f)),
        ExprKind::Map(entries) => entries.iter_mut().for_each(|(k, v)| {
            visit_expr(k, f);
            visit_expr(v, f);
        }),
        ExprKind::Record { fields, .. } => fields.iter_mut().for_each(|(_, e)| visit_expr(e, f)),
        ExprKind::With(base, fields) => {
            visit_expr(base, f);
            fields.iter_mut().for_each(|(_, e)| visit_expr(e, f));
        }
        ExprKind::Builtin { args, named, .. } => {
            args.iter_mut().for_each(|e| visit_expr(e, f));
            named.iter_mut().for_each(|(_, e)| visit_expr(e, f));
        }
        ExprKind::Closure { func, capture_args } | ExprKind::Simulate { func, capture_args } => {
            f(&mut CallSite::Closure(*func, capture_args))
        }
        ExprKind::Interp(parts) => parts.iter_mut().for_each(|p| {
            if let InterpPart::Expr(e) = p {
                visit_expr(e, f)
            }
        }),
    }
}

// ── Small constructors ───────────────────────────────────────────────────

fn lit(l: Lit, span: Span) -> Expr {
    Expr {
        kind: ExprKind::Lit(l),
        span,
    }
}

fn slot(s: SlotId, span: Span) -> Expr {
    Expr {
        kind: ExprKind::Slot(s),
        span,
    }
}

fn binary(op: BinOp, a: Expr, b: Expr, span: Span) -> Expr {
    Expr {
        kind: ExprKind::Binary(op, Box::new(a), Box::new(b)),
        span,
    }
}

fn builtin(func: Builtin, args: Vec<Expr>, span: Span) -> Expr {
    Expr {
        kind: ExprKind::Builtin {
            func,
            args,
            named: Vec::new(),
        },
        span,
    }
}

/// An expression reading the current value of a place.
fn place_read(place: &Place, span: Span) -> Expr {
    let mut e = slot(place.slot, span);
    for elem in &place.path {
        e = Expr {
            kind: match elem {
                PathElem::Field(name) => ExprKind::Field(Box::new(e), name.clone()),
                PathElem::Index(i) => ExprKind::Index(Box::new(e), Box::new(i.clone())),
            },
            span,
        };
    }
    e
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn pragma_int(e: &ast::Expr) -> Option<u64> {
    match e.kind {
        ast::ExprKind::Int(v) if v >= 0 => Some(v as u64),
        _ => None,
    }
}

fn pragma_float(e: &ast::Expr) -> Option<f64> {
    match e.kind {
        ast::ExprKind::Int(v) => Some(v as f64),
        ast::ExprKind::Float(v) => Some(v),
        _ => None,
    }
}

/// The most similar name, if one is close enough to be a likely typo.
fn closest(name: &str, candidates: &[String]) -> Option<String> {
    let limit = match name.len() {
        0..=3 => 1,
        4..=7 => 2,
        _ => 3,
    };
    candidates
        .iter()
        .filter(|c| c.as_str() != name && !c.starts_with('$'))
        .map(|c| (edit_distance(name, c), c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d) // the first of equals: candidates come nearest-first
        .map(|(_, c)| c.clone())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}
