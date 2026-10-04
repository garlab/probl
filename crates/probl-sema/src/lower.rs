//! Name resolution and lowering from the AST to the IR.
//!
//! The pass resolves names to frame slots, checks the language's static rules
//! (functions only assign their own variables, `report` only at the top level,
//! …), and moves everything that can split worlds out of expressions into
//! statements of its own.

use crate::builtins::{Builtin, Constant};
use crate::ir::*;
use crate::symbols::{DefKind, Definition, Symbols};
use probl_syntax::ast::{self, BinOp, UnOp};
use probl_syntax::{Diagnostic, Span};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{BTreeMap, BTreeSet};

/// Lower a parsed program. The IR is returned even when there are errors, but
/// it must not be run unless all diagnostics are warnings.
pub fn lower(program: &ast::Program, src: &str) -> (Program, Vec<Diagnostic>) {
    let (program, diags, _) = lower_with_symbols(program, src);
    (program, diags)
}

/// Lower a parsed program, and say where its names are declared and used.
pub fn lower_with_symbols(program: &ast::Program, src: &str) -> (Program, Vec<Diagnostic>, Symbols) {
    let mut lowerer = Lowerer::new(src);
    lowerer.program(program);
    let mut symbols = std::mem::take(&mut lowerer.symbols);
    // A field read by name is the one declared record's with that field, if
    // only one has it and no record without a type does. Otherwise it
    // depends on the value.
    for (span, name) in std::mem::take(&mut lowerer.field_uses) {
        if lowerer.anonymous_fields.contains(&name) {
            continue;
        }
        if let Some(&[(_, def)]) = lowerer.field_defs.get(&name).map(Vec::as_slice) {
            symbols.references.push((span, def));
        }
    }
    // Scopes still open when lowering ended run to the end of the program.
    let end = src.len() as u32;
    for d in &mut symbols.definitions {
        d.scope.hi = d.scope.hi.min(end);
    }
    let (program, diags) = lowerer.finish();
    (program, diags, symbols)
}

/// The name of compiler-generated variables. Each is assigned once per path
/// and never changed afterwards.
const TEMP: &str = "(temporary)";

struct FnBuild {
    name: String,
    kind: FnKind,
    span: Span,
    n_params: u32,
    param_types: Vec<Option<TypeSpec>>,
    /// Expected types for branch-result temporaries, used only while lowering.
    result_types: FxHashMap<SlotId, TypeSpec>,
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

#[derive(Clone)]
struct Binding {
    slot: SlotId,
    /// Its definition in `Lowerer::symbols`.
    def: usize,
    mutable: bool,
    /// Copied in from outside the function, so it can't be assigned.
    captured: bool,
    /// Declared type, checked whenever the variable is assigned.
    ty: Option<TypeSpec>,
}

struct Ctx {
    func: FnId,
    scopes: Vec<FxHashMap<String, Binding>>,
    /// Enclosing loops, innermost last.
    loops: Vec<LoopKind>,
    /// The declared return type, checked at every `return`.
    ret: Option<TypeSpec>,
}

#[derive(Clone, Debug, PartialEq)]
enum LoopKind {
    /// A `for` loop; its binding slot only when iteration keys are provably unique.
    For(Option<SlotId>),
    Other,
}

/// What the statements lowered from a later operand can do. It decides which
/// earlier operands must be evaluated before them (docs/semantics.md,
/// section 4).
#[derive(Clone, Copy)]
struct Later {
    /// There are statements: they can split worlds, observe, fail or print.
    any: bool,
    /// They can assign a program variable.
    assigns: bool,
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
    inputs: Vec<Input>,
    /// Where standard input is read, if it is: only once.
    stdin: Option<Span>,
    settings: Settings,
    next_stmt: u32,
    ctx: Vec<Ctx>,
    /// Where names are declared and used.
    symbols: Symbols,
    /// The definitions in each open scope, innermost last, whose scope ends
    /// with it.
    open: Vec<Vec<usize>>,
    fn_defs: FxHashMap<String, usize>,
    type_defs: FxHashMap<String, usize>,
    variant_defs: FxHashMap<(u32, u32), usize>,
    /// The declared records' fields by name: their record and definition.
    field_defs: FxHashMap<String, Vec<(u32, usize)>>,
    /// The names of fields in records without a declared type.
    anonymous_fields: FxHashSet<String>,
    /// Fields used by name, whose record isn't known until the program runs.
    field_uses: Vec<(Span, String)>,
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
            inputs: Vec::new(),
            stdin: None,
            settings: Settings::default(),
            next_stmt: 0,
            ctx: Vec::new(),
            symbols: Symbols::default(),
            open: Vec::new(),
            fn_defs: FxHashMap::default(),
            type_defs: FxHashMap::default(),
            variant_defs: FxHashMap::default(),
            field_defs: FxHashMap::default(),
            anonymous_fields: FxHashSet::default(),
            field_uses: Vec::new(),
        }
    }

    /// Record a definition, visible from `scope.lo` to `scope.hi`.
    fn define(&mut self, name: &str, kind: DefKind, span: Span, scope: Span) -> usize {
        self.symbols.definitions.push(Definition {
            name: name.to_string(),
            kind,
            mutable: false,
            span,
            scope,
            global: false,
            owner: None,
            ty: None,
        });
        self.symbols.definitions.len() - 1
    }

    /// Record a use of a name.
    fn refer(&mut self, span: Span, def: usize) {
        self.symbols.references.push((span, def));
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
            param_types: Vec::new(),
            result_types: FxHashMap::default(),
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
        self.new_slot_in(func, TEMP, span)
    }

    fn stmt(&mut self, span: Span, kind: StmtKind) -> Stmt {
        let id = self.next_stmt;
        self.next_stmt += 1;
        Stmt { id, span, kind }
    }

    fn push_scope(&mut self) {
        self.cur().scopes.push(FxHashMap::default());
        self.open.push(Vec::new());
    }

    /// Close the innermost scope, which ends in the source at `end`.
    fn pop_scope(&mut self, end: u32) {
        self.cur().scopes.pop();
        for d in self.open.pop().unwrap_or_default() {
            self.symbols.definitions[d].scope.hi = end;
        }
    }

    fn at_top_level(&self) -> bool {
        self.ctx.len() == 1 && self.ctx[0].func == MAIN && self.ctx[0].scopes.len() == 1
    }

    /// Declare a variable in the innermost scope.
    fn declare(&mut self, name: &str, span: Span, mutable: bool) -> SlotId {
        self.declare_as(name, span, mutable, DefKind::Variable).0
    }

    /// Declare a variable or a parameter in the innermost scope, and return
    /// its slot and its definition. A parameter's scope is set by its
    /// function.
    fn declare_as(&mut self, name: &str, span: Span, mutable: bool, kind: DefKind) -> (SlotId, usize) {
        let top = self.at_top_level();
        let def = self.define(name, kind, span, Span::new(span.hi as usize, u32::MAX as usize));
        self.symbols.definitions[def].mutable = mutable;
        if top {
            self.symbols.definitions[def].global = true;
        } else if kind == DefKind::Variable {
            if let Some(open) = self.open.last_mut() {
                open.push(def);
            }
        }
        if top && self.globals.contains_key(name) {
            self.error(span, format!("`{name}` is already defined at the top level"))
                .help("top-level names must be unique; use `var` and assign to change a value");
        }
        let func = self.cur_func();
        let slot = self.new_slot_in(func, name, span);
        let binding = Binding {
            slot,
            def,
            mutable,
            captured: false,
            ty: None,
        };
        if top {
            self.globals.insert(name.to_string(), binding.clone());
        }
        self.cur().scopes.last_mut().unwrap().insert(name.to_string(), binding);
        (slot, def)
    }

    /// Record a declared type for the variable most recently declared as `name`.
    fn set_declared_type(&mut self, name: &str, ty: &TypeSpec) {
        if let Some(b) = self.cur().scopes.last_mut().unwrap().get_mut(name) {
            b.ty = Some(ty.clone());
        }
    }

    fn check(&mut self, slot_id: SlotId, ty: &TypeSpec, span: Span) -> Stmt {
        self.stmt(
            span,
            StmtKind::Check {
                slot: slot_id,
                ty: ty.clone(),
            },
        )
    }

    /// The type named by a type annotation.
    fn type_spec(&mut self, t: &ast::TypeExpr) -> Option<TypeSpec> {
        match t {
            ast::TypeExpr::Record { fields, .. } => {
                let mut specs = Vec::new();
                for (name, ty) in fields {
                    self.anonymous_fields.insert(name.name.clone());
                    specs.push((name.name.clone(), self.type_spec(ty)?));
                }
                specs.sort_by(|a, b| a.0.cmp(&b.0));
                Some(TypeSpec::AnonRecord(specs))
            }
            ast::TypeExpr::Named { name, args } => {
                let n = name.name.as_str();
                let expected = match n {
                    "list" | "bag" | "dist" => 1,
                    "map" => 2,
                    _ => 0,
                };
                if args.len() != expected {
                    let msg = if expected == 0 {
                        format!("`{n}` doesn't take type arguments")
                    } else {
                        format!(
                            "`{n}` takes {expected} type argument{}, like `{n}[int]`",
                            plural(expected)
                        )
                    };
                    self.error(name.span, msg);
                    return None;
                }
                let arg = |i: usize, this: &mut Self| this.type_spec(&args[i]).map(Box::new);
                Some(match n {
                    "int" => TypeSpec::Int,
                    "float" => TypeSpec::Float,
                    "complex" => TypeSpec::Complex,
                    "prob" => TypeSpec::Prob,
                    "bool" => TypeSpec::Bool,
                    "str" => TypeSpec::Str,
                    "date" => TypeSpec::Date,
                    "list" => TypeSpec::List(arg(0, self)?),
                    "bag" => TypeSpec::Bag(arg(0, self)?),
                    "dist" => TypeSpec::Dist(arg(0, self)?),
                    "map" => TypeSpec::Map(arg(0, self)?, arg(1, self)?),
                    _ => {
                        if let Some(&def) = self.type_defs.get(n) {
                            self.refer(name.span, def);
                        }
                        if let Some(&r) = self.record_by_name.get(n) {
                            TypeSpec::Record(r)
                        } else if let Some(&e) = self.enum_by_name.get(n) {
                            TypeSpec::Enum(e)
                        } else {
                            let mut known: Vec<String> = [
                                "int", "float", "complex", "prob", "bool", "str", "date", "list", "map", "bag", "dist",
                            ]
                            .iter()
                            .map(|s| s.to_string())
                            .collect();
                            known.extend(self.record_by_name.keys().cloned());
                            known.extend(self.enum_by_name.keys().cloned());
                            let suggestion = closest(n, &known);
                            let d = self.error(name.span, format!("unknown type `{n}`"));
                            if let Some(s) = suggestion {
                                d.help(format!("did you mean `{s}`?"));
                            }
                            return None;
                        }
                    }
                })
            }
        }
    }

    /// Reject a literal that can never have the declared type.
    fn check_literal(&mut self, value: &ast::Expr, ty: &TypeSpec) {
        let found = match (&value.kind, ty) {
            (ast::ExprKind::Int(v), TypeSpec::Prob) if *v == 0 || *v == 1 => return,
            (ast::ExprKind::Int(_), TypeSpec::Int | TypeSpec::Float) => return,
            // Contextualization below diagnoses fractional literals precisely.
            (ast::ExprKind::Float(_) | ast::ExprKind::Percent(_), TypeSpec::Int) => return,
            (ast::ExprKind::Float(v), TypeSpec::Prob) if (0.0..=1.0).contains(v) => return,
            (ast::ExprKind::Float(_), TypeSpec::Float) => return,
            (ast::ExprKind::Percent(v), TypeSpec::Prob) if (0.0..=1.0).contains(v) => return,
            (ast::ExprKind::Percent(_), TypeSpec::Float) => return,
            (ast::ExprKind::Str(_), TypeSpec::Str) => return,
            (ast::ExprKind::Bool(_), TypeSpec::Bool) => return,
            (ast::ExprKind::Int(_), _) => "an int",
            (ast::ExprKind::Float(_), _) => "a float",
            (ast::ExprKind::Percent(_), _) => "a percentage",
            (ast::ExprKind::Str(_), _) => "a string",
            (ast::ExprKind::Bool(_), _) => "a bool",
            _ => return,
        };
        let expected = match ty {
            TypeSpec::Int => "an int",
            TypeSpec::Float => "a float",
            TypeSpec::Complex => "a complex",
            TypeSpec::Prob => "a probability",
            TypeSpec::Bool => "a bool",
            TypeSpec::Str => "a string",
            TypeSpec::Date => "a date",
            _ => "a value of the declared type",
        };
        self.error(value.span, format!("expected {expected}, found {found}"));
    }

    /// Convert known numeric literals early, diagnosing invalid ranges.
    /// Other numbers are checked at the receiving runtime type boundary.
    fn contextualize(&mut self, e: &mut Expr, ty: &TypeSpec) {
        if *ty == TypeSpec::Int {
            if let Some(n) = crate::coercions::float_literal(e) {
                if !n.is_finite() || n.fract() != 0.0 {
                    self.error(
                        e.span,
                        "an int needs an exactly integral finite float; conversion never rounds",
                    );
                }
            }
            // Runtime conversion accounts for integer allocation and host limits.
            return;
        }
        if *ty == TypeSpec::Prob {
            if let Some(p) = crate::coercions::numeric_literal(e) {
                if p.is_finite() && (0.0..=1.0).contains(&p) {
                    e.kind = ExprKind::Lit(Lit::Prob(p));
                } else {
                    self.error(e.span, "a probability literal must be between 0 and 1");
                }
            }
            return;
        }
        match (&mut e.kind, ty) {
            (ExprKind::List(xs), TypeSpec::List(t)) => {
                for x in xs {
                    self.contextualize(x, t);
                }
            }
            (ExprKind::Map(xs), TypeSpec::Map(_, v)) => {
                // Convert keys only at the map boundary, where collisions can
                // be detected before an entry is overwritten.
                for (_, b) in xs {
                    self.contextualize(b, v);
                }
            }
            (ExprKind::Record { fields, .. }, TypeSpec::AnonRecord(ts)) => {
                for (n, x) in fields {
                    if let Some((_, t)) = ts.iter().find(|(m, _)| m == n) {
                        self.contextualize(x, t);
                    }
                }
            }
            _ => {}
        }
    }

    /// Resolve a variable name from the context at `depth`, capturing it into
    /// lambdas and functions as needed.
    fn lookup_at(&mut self, depth: usize, name: &str) -> Option<Binding> {
        for scope in self.ctx[depth].scopes.iter().rev() {
            if let Some(b) = scope.get(name) {
                return Some(b.clone());
            }
        }
        let func = self.ctx[depth].func;
        let kind = self.funcs[func as usize].kind;
        let (found, def) = match kind {
            FnKind::Main => return None,
            FnKind::Lambda | FnKind::Simulate if depth > 0 => {
                let outer = self.lookup_at(depth - 1, name)?;
                let span = self.funcs[func as usize].span;
                let slot = self.new_slot_in(func, name, span);
                self.funcs[func as usize].parent_captures.push((slot, outer.slot));
                (slot, outer.def)
            }
            _ => {
                let global = self.globals.get(name)?.clone();
                let span = self.funcs[func as usize].span;
                let slot = self.new_slot_in(func, name, span);
                self.funcs[func as usize].global_slots.insert(global.slot, slot);
                (slot, global.def)
            }
        };
        let binding = Binding {
            slot: found,
            def,
            mutable: false,
            captured: true,
            ty: None,
        };
        self.ctx[depth].scopes[0].insert(name.to_string(), binding.clone());
        Some(binding)
    }

    fn lookup(&mut self, name: &str) -> Option<Binding> {
        let depth = self.ctx.len() - 1;
        self.lookup_at(depth, name)
    }

    /// Record a use of an enum's variant.
    fn refer_variant(&mut self, variant: &Lit, span: Span) {
        if let Lit::Enum { ty, variant } = variant {
            if let Some(&def) = self.variant_defs.get(&(*ty, *variant)) {
                self.refer(span, def);
            }
        }
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
        names.extend(Constant::ALL.iter().map(|c| c.name().to_string()));
        names.push("read".to_string());
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
        let mut type_decls = Vec::new();
        for item in &program.items {
            match item {
                ast::Item::Fn(decl) => {
                    let name = &decl.name.name;
                    if self.fn_by_name.contains_key(name) {
                        self.error(decl.name.span, format!("the function `{name}` is defined twice"));
                        continue;
                    }
                    if Builtin::from_name(name).is_some() || name == "read" {
                        self.warning(
                            decl.name.span,
                            format!("`{name}` hides the built-in function of the same name"),
                        );
                    }
                    let id = self.new_fn(name, FnKind::Named, decl.span, None);
                    self.funcs[id as usize].n_params = decl.params.len() as u32;
                    self.fn_by_name.insert(name.clone(), id);
                    let everywhere = Span::new(0, u32::MAX as usize);
                    let def = self.define(name, DefKind::Function, decl.name.span, everywhere);
                    self.fn_defs.insert(name.clone(), def);
                    fn_decls.push((id, decl));
                }
                ast::Item::Type(decl) => {
                    if let Some(id) = self.type_decl(decl) {
                        type_decls.push((id, decl));
                    }
                }
                ast::Item::Enum(decl) => self.enum_decl(decl),
                ast::Item::Import(import) => {
                    self.error(import.span, "`import` isn't supported yet");
                }
                ast::Item::Stmt(_) => {}
            }
        }
        // Fields' types, once every type's name is known.
        for (id, decl) in type_decls {
            self.record_fields(id, decl);
        }
        self.check_record_cycles();
        for (id, decl) in &fn_decls {
            self.funcs[*id as usize].param_types = decl
                .params
                .iter()
                .map(|p| p.ty.as_ref().and_then(|t| self.type_spec(t)))
                .collect();
        }

        // The top level.
        self.ctx.push(Ctx {
            func: MAIN,
            scopes: vec![FxHashMap::default()],
            loops: Vec::new(),
            ret: None,
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
                    "enumerate" => self.settings.mode = Mode::Enumerate,
                    "auto" => self.settings.mode = Mode::Auto,
                    "exact" => {
                        self.error(pragma.span, "`@mode exact` is now called `@mode enumerate`")
                            .note("enumeration follows every branch, but its answers are floating-point and loops may be cut short, so it isn't called exact");
                    }
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
            "use `@mode enumerate`, `@mode auto`, `@mode sample(runs: 10_000, seed: 1)`, \
             `@mode particles(runs: 10_000, seed: 1)` or `@mode beam(worlds: 100_000)`",
        );
    }

    /// Register a record type's name; its fields come later, once every
    /// type's name is known (`record_fields`).
    fn type_decl(&mut self, decl: &ast::TypeDecl) -> Option<u32> {
        let name = &decl.name.name;
        if self.record_by_name.contains_key(name) || self.enum_by_name.contains_key(name) {
            self.error(decl.name.span, format!("the type `{name}` is defined twice"));
            return None;
        }
        if !matches!(decl.ty, ast::TypeExpr::Record { .. }) {
            self.error(decl.span, "only record types can be declared for now")
                .help("write the fields in braces: `type Point = { x: int, y: int }`");
            return None;
        }
        let id = self.records.len() as u32;
        self.record_by_name.insert(name.clone(), id);
        let def = self.define(name, DefKind::Record, decl.name.span, Span::new(0, u32::MAX as usize));
        self.type_defs.insert(name.clone(), def);
        self.records.push(RecordType {
            name: name.clone(),
            fields: Vec::new(),
            span: decl.name.span,
        });
        Some(id)
    }

    /// Resolve a record type's fields and their types.
    fn record_fields(&mut self, id: u32, decl: &ast::TypeDecl) {
        let ast::TypeExpr::Record { fields, .. } = &decl.ty else {
            return;
        };
        let mut resolved: Vec<RecordField> = Vec::new();
        for (field, ty) in fields {
            if resolved.iter().any(|f| f.name == field.name) {
                self.error(field.span, format!("the field `{}` appears twice", field.name));
                continue;
            }
            // An unknown type has been reported; the program won't run.
            let ty = self.type_spec(ty).unwrap_or(TypeSpec::Unit);
            let def = self.define(&field.name, DefKind::Field, field.span, Span::new(0, u32::MAX as usize));
            self.symbols.definitions[def].owner = Some(self.records[id as usize].name.clone());
            self.symbols.definitions[def].ty = Some(ty.describe_in(&self.records, &self.enums));
            self.field_defs.entry(field.name.clone()).or_default().push((id, def));
            resolved.push(RecordField {
                name: field.name.clone(),
                ty,
                span: field.span,
            });
        }
        self.records[id as usize].fields = resolved;
    }

    /// A record type that contains itself, other than inside a list, map or
    /// bag, has no values: each would contain another one forever.
    fn check_record_cycles(&mut self) {
        fn needs(ty: &TypeSpec, out: &mut Vec<u32>) {
            match ty {
                TypeSpec::Record(r) => out.push(*r),
                TypeSpec::Dist(t) => needs(t, out),
                TypeSpec::AnonRecord(fields) => fields.iter().for_each(|(_, t)| needs(t, out)),
                _ => {}
            }
        }
        let edges: Vec<Vec<u32>> = self
            .records
            .iter()
            .map(|r| {
                let mut out = Vec::new();
                r.fields.iter().for_each(|f| needs(&f.ty, &mut out));
                out
            })
            .collect();
        for start in 0..self.records.len() {
            let mut seen = vec![false; self.records.len()];
            let mut stack = edges[start].clone();
            let mut cyclic = false;
            while let Some(r) = stack.pop() {
                if r as usize == start {
                    cyclic = true;
                    break;
                }
                if !std::mem::replace(&mut seen[r as usize], true) {
                    stack.extend(&edges[r as usize]);
                }
            }
            if cyclic {
                let (name, span) = (self.records[start].name.clone(), self.records[start].span);
                self.error(span, format!("every `{name}` would contain another `{name}`, forever"))
                    .help(format!(
                        "hold the inner ones in a list, which can be empty, like `parts: list[{name}]`"
                    ));
            }
        }
    }

    fn enum_decl(&mut self, decl: &ast::EnumDecl) {
        let name = &decl.name.name;
        if self.record_by_name.contains_key(name) || self.enum_by_name.contains_key(name) {
            self.error(decl.name.span, format!("the type `{name}` is defined twice"));
            return;
        }
        let ty = self.enums.len() as u32;
        let everywhere = Span::new(0, u32::MAX as usize);
        let def = self.define(name, DefKind::Enum, decl.name.span, everywhere);
        self.type_defs.insert(name.clone(), def);
        let mut variants = Vec::new();
        for (i, variant) in decl.variants.iter().enumerate() {
            if variants.contains(&variant.name) {
                self.error(variant.span, format!("the variant `{}` appears twice", variant.name));
            }
            let def = self.define(&variant.name, DefKind::Variant, variant.span, everywhere);
            self.symbols.definitions[def].owner = Some(name.clone());
            self.variant_defs.insert((ty, i as u32), def);
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
        self.symbols.functions.push(decl.span);
        self.ctx.push(Ctx {
            func: id,
            scopes: vec![FxHashMap::default()],
            loops: Vec::new(),
            ret: None,
        });
        let mut stmts = Vec::new();
        for param in &decl.params {
            if self.ctx[self.ctx.len() - 1].scopes[0].contains_key(&param.name.name) {
                self.error(
                    param.name.span,
                    format!("the parameter `{}` appears twice", param.name.name),
                );
            }
            let (slot_id, def) = self.declare_as(&param.name.name, param.name.span, false, DefKind::Parameter);
            self.symbols.definitions[def].scope.hi = decl.span.hi;
            if let Some(spec) = param.ty.as_ref().and_then(|t| self.type_spec(t)) {
                self.set_declared_type(&param.name.name, &spec);
                let st = self.check(slot_id, &spec, param.name.span);
                stmts.push(st);
            }
        }
        self.funcs[id as usize].n_params = decl.params.len() as u32;
        self.cur().ret = decl.ret.as_ref().and_then(|t| self.type_spec(t));
        self.push_scope();
        let ret_type = self.cur().ret.clone();
        let value = self.block_value_expected(&decl.body, ret_type.as_ref(), &mut stmts);
        self.pop_scope(decl.body.span.hi);
        let value = self.checked_return_value(value, decl.body.span, &mut stmts);
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
                ty,
            } => {
                if let Some(args) = self.read_call(value) {
                    self.read_binding(*mutable, pattern, *op, ty.as_ref(), args, value.span, s.span, out);
                    return;
                }
                let spec = ty.as_ref().and_then(|t| self.type_spec(t));
                if let Some(spec) = &spec {
                    if *op == ast::BindOp::Assign {
                        self.check_literal(value, spec);
                    }
                }
                self.let_stmt(*mutable, pattern, *op, value, spec.as_ref(), s.span, out);
                if let (Some(spec), Some(_)) = (spec, ty) {
                    match &pattern.kind {
                        ast::PatternKind::Name(name) => {
                            let slot_id = self.lookup(name).unwrap().slot;
                            self.set_declared_type(name, &spec);
                            let st = self.check(slot_id, &spec, s.span);
                            out.push(st);
                        }
                        _ => {
                            self.error(pattern.span, "a type annotation needs a plain variable name");
                        }
                    }
                }
            }
            ast::StmtKind::Assign { target, op, value } => self.assign(target, *op, value, s.span, out),
            ast::StmtKind::For { pattern, iter, body } => self.for_loop(pattern, iter, body, s.span, out),
            ast::StmtKind::While { cond, body } => {
                // loop { if cond { body } else { break } }
                self.cur().loops.push(LoopKind::Other);
                let mut inner = Vec::new();
                let c = self.expr_expected(cond, Some(&TypeSpec::Prob), &mut inner);
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
                self.cur().loops.pop();
                let lp = self.stmt(
                    s.span,
                    StmtKind::Loop {
                        body: Block { stmts: inner },
                        bounded: false,
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
                        value: lit(Lit::Int((0).into()), s.span),
                    },
                );
                out.push(set_n);
                out.push(set_k);
                self.cur().loops.push(LoopKind::Other);
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
                        value: binary(BinOp::Add, slot(k, s.span), lit(Lit::Int((1).into()), s.span), s.span),
                    },
                );
                inner.push(incr);
                let body = self.scoped_block(body);
                inner.extend(body.stmts);
                self.cur().loops.pop();
                let lp = self.stmt(
                    s.span,
                    StmtKind::Loop {
                        body: Block { stmts: inner },
                        bounded: true,
                    },
                );
                out.push(lp);
            }
            ast::StmtKind::Loop { body } => {
                self.cur().loops.push(LoopKind::Other);
                let body = self.scoped_block(body);
                self.cur().loops.pop();
                let lp = self.stmt(s.span, StmtKind::Loop { body, bounded: false });
                out.push(lp);
            }
            ast::StmtKind::Break | ast::StmtKind::Continue => {
                let is_break = matches!(s.kind, ast::StmtKind::Break);
                if self.ctx.last().unwrap().loops.is_empty() {
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
                let ret_type = self.cur().ret.clone();
                let v = match value {
                    Some(v) => self.expr_expected(v, ret_type.as_ref(), out),
                    None => lit(Lit::Unit, s.span),
                };
                let v = self.checked_return_value(v, s.span, out);
                let st = self.stmt(s.span, StmtKind::Return(v));
                out.push(st);
            }
            ast::StmtKind::Observe { value, from } => {
                // An immediately observed anonymous draw can be integrated
                // out. Preserve missing mass and reject non-boolean laws.
                if from.is_none() {
                    if let ast::ExprKind::Draw(source) = &value.kind {
                        // Operand effects, including bag extraction, still
                        // execute before the anonymous trial is integrated.
                        let d = self.expr(source, out);
                        let d = builtin(Builtin::BooleanLaw, vec![d], source.span);
                        let st = self.stmt(
                            s.span,
                            StmtKind::Observe {
                                value: lit(Lit::Bool(true), value.span),
                                from: Some(d),
                            },
                        );
                        out.push(st);
                        return;
                    }
                }
                let mut exprs = vec![value];
                exprs.extend(from.as_ref());
                let mut values = self.operands(&exprs, out).into_iter();
                let v = values.next().unwrap();
                let d = values.next();
                let st = self.stmt(s.span, StmtKind::Observe { value: v, from: d });
                out.push(st);
            }
            ast::StmtKind::Score(value) => {
                let p = self.expr_expected(value, Some(&TypeSpec::Prob), out);
                let d = builtin(Builtin::ScoreLaw, vec![p], value.span);
                let st = self.stmt(
                    s.span,
                    StmtKind::Observe {
                        value: lit(Lit::Bool(true), value.span),
                        from: Some(d),
                    },
                );
                out.push(st);
            }
            ast::StmtKind::Report { value, by, label } => self.report(value, by.as_ref(), label, s.span, out),
            ast::StmtKind::Expr(e) => self.expr_stmt(e, out),
        }
    }

    /// With a declared return type, store the value and check it first.
    fn checked_return_value(&mut self, mut value: Expr, span: Span, out: &mut Vec<Stmt>) -> Expr {
        let Some(ty) = self.ctx.last().unwrap().ret.clone() else {
            return value;
        };
        self.contextualize(&mut value, &ty);
        let t = self.temp(span);
        let set = self.stmt(
            span,
            StmtKind::Set {
                place: Place::slot(t),
                value,
            },
        );
        out.push(set);
        let st = self.check(t, &ty, span);
        out.push(st);
        slot(t, span)
    }

    fn scoped_block(&mut self, block: &ast::Block) -> Block {
        self.push_scope();
        let mut stmts = Vec::new();
        for s in &block.stmts {
            self.lower_stmt(s, &mut stmts);
        }
        self.pop_scope(block.span.hi);
        Block { stmts }
    }

    /// Lower the statements of a block and return the value of its last
    /// expression (or `()`). The caller manages the scope.
    fn block_value(&mut self, block: &ast::Block, out: &mut Vec<Stmt>) -> Expr {
        self.block_value_expected(block, None, out)
    }

    fn block_value_expected(&mut self, block: &ast::Block, ty: Option<&TypeSpec>, out: &mut Vec<Stmt>) -> Expr {
        let Some((last, init)) = block.stmts.split_last() else {
            return lit(Lit::Unit, block.span);
        };
        for s in init {
            self.lower_stmt(s, out);
        }
        match &last.kind {
            ast::StmtKind::Expr(e) => self.expr_expected(e, ty, out),
            _ => {
                self.lower_stmt(last, out);
                lit(Lit::Unit, last.span)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn let_stmt(
        &mut self,
        mutable: bool,
        pattern: &ast::Pattern,
        op: ast::BindOp,
        value: &ast::Expr,
        ty: Option<&TypeSpec>,
        span: Span,
        out: &mut Vec<Stmt>,
    ) {
        if op == ast::BindOp::Assign {
            if let ast::ExprKind::Draw(source) = &value.kind {
                return self.let_stmt(mutable, pattern, ast::BindOp::Draw, source, ty, span, out);
            }
        }
        let v = self.expr_expected(value, if op == ast::BindOp::Assign { ty } else { None }, out);
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

    /// The arguments of `read(…)`, if `value` is a call to it.
    fn read_call<'e>(&self, value: &'e ast::Expr) -> Option<&'e [ast::Arg]> {
        match &value.kind {
            ast::ExprKind::Call { callee, args }
                if matches!(&callee.kind, ast::ExprKind::Name(n) if n == "read")
                    && !self.fn_by_name.contains_key("read") =>
            {
                Some(args)
            }
            _ => None,
        }
    }

    /// `let name: T = read("path")`: add an input to the program's manifest
    /// and bind its value (docs/data-input.md). After an error, the variable
    /// is still declared, so that its uses don't fail too.
    #[allow(clippy::too_many_arguments)]
    fn read_binding(
        &mut self,
        mutable: bool,
        pattern: &ast::Pattern,
        op: ast::BindOp,
        ty: Option<&ast::TypeExpr>,
        args: &[ast::Arg],
        call: Span,
        span: Span,
        out: &mut Vec<Stmt>,
    ) {
        let ast::PatternKind::Name(name) = &pattern.kind else {
            self.error(
                pattern.span,
                "data is bound to a variable name, like `let rows: list[Row] = read(…)`",
            );
            return;
        };
        let input = self.read_input(name, op, ty, args, call);
        let dest = self.declare(name, pattern.span, mutable);
        let value = match input {
            Some(input) => {
                self.set_declared_type(name, &input.ty);
                self.inputs.push(input);
                ExprKind::Input(self.inputs.len() as u32 - 1)
            }
            None => ExprKind::Lit(Lit::Unit),
        };
        let st = self.stmt(
            span,
            StmtKind::Set {
                place: Place::slot(dest),
                value: Expr {
                    kind: value,
                    span: call,
                },
            },
        );
        out.push(st);
    }

    /// Check a `read` and describe what it reads.
    fn read_input(
        &mut self,
        name: &str,
        op: ast::BindOp,
        ty: Option<&ast::TypeExpr>,
        args: &[ast::Arg],
        call: Span,
    ) -> Option<Input> {
        let mut ok = true;
        if !self.at_top_level() {
            self.error(call, "`read` must be at the top level of the program")
                .help("data is read once, before the program runs: read it at the top, and use the variable here");
            ok = false;
        }
        if op == ast::BindOp::Draw {
            self.error(call, "data is a value, so it's bound with `=`, not `~`");
            ok = false;
        }
        // One path, written out, and an optional format.
        let literal = |e: &ast::Expr| match &e.kind {
            ast::ExprKind::Str(parts) => parts
                .iter()
                .map(|p| match p {
                    ast::StrSegment::Lit(s) => Some(s.as_str()),
                    ast::StrSegment::Expr(_) => None,
                })
                .collect::<Option<String>>(),
            _ => None,
        };
        let (mut path, mut format) = (None, None);
        for arg in args {
            match &arg.name {
                None if path.is_none() => path = Some(&arg.value),
                None => {
                    self.error(arg.value.span, "`read` takes one path");
                    ok = false;
                }
                Some(n) if n.name == "format" && format.is_none() => format = Some(&arg.value),
                Some(n) => {
                    self.error(n.span, format!("`read` has no argument `{}`", n.name))
                        .help("it takes a path, and a `format:` when the path's extension doesn't say");
                    ok = false;
                }
            }
        }
        let Some(path_expr) = path else {
            self.error(call, "`read` needs the path of the data, like `read(\"rows.csv\")`");
            return None;
        };
        let Some(path) = literal(path_expr).filter(|p| !p.is_empty()) else {
            self.error(path_expr.span, "the path must be written out, as a string")
                .help("the data is read before the program runs, so the path can't be computed");
            return None;
        };
        let format = match format {
            Some(f) => match literal(f).as_deref().and_then(DataFormat::from_name) {
                Some(format) => format,
                None => {
                    self.error(f.span, "the format is \"csv\", \"json\" or \"lines\"");
                    return None;
                }
            },
            None => match DataFormat::from_path(&path) {
                Some(format) => format,
                None => {
                    self.error(
                        path_expr.span,
                        format!("can't tell the format of `{path}` from its name"),
                    )
                    .help(format!(
                        "say it, like `read(\"{path}\", format: \"csv\")`: the formats are csv, json and lines"
                    ));
                    return None;
                }
            },
        };
        if path == "-" {
            if self.stdin.is_some() {
                self.error(call, "standard input can only be read once")
                    .help("read it into one variable, and use that variable");
                ok = false;
            }
            self.stdin = Some(call);
        }
        let Some(ty) = ty else {
            let example = match format {
                DataFormat::Csv => "list[Row]",
                DataFormat::Json => "Settings",
                DataFormat::Lines => "list[int]",
            };
            self.error(call, format!("say what the data is: `let {name}: {example} = read(…)`"))
                .help("the type decides how the data is read; `probl schema FILE` suggests one");
            return None;
        };
        let spec = self.type_spec(ty)?;
        for problem in crate::data::check(&spec, format, &self.records, &self.enums) {
            let d = self.error(problem.at.unwrap_or(type_span(ty)), problem.message);
            if let Some(help) = problem.help {
                d.help(help);
            }
            ok = false;
        }
        ok.then(|| Input {
            name: name.to_string(),
            path,
            format,
            ty: spec,
            span: call,
        })
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
                    vec![value.clone(), lit(Lit::Int((items.len() as i64).into()), pattern.span)],
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
                        kind: ExprKind::Index(
                            Box::new(value.clone()),
                            Box::new(lit(Lit::Int((i as i64).into()), item.span)),
                        ),
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

    fn assign(&mut self, target: &ast::Expr, op: ast::AssignOp, value: &ast::Expr, span: Span, out: &mut Vec<Stmt>) {
        if op == ast::AssignOp::Set {
            if let ast::ExprKind::Draw(source) = &value.kind {
                return self.assign(target, ast::AssignOp::Draw, source, span, out);
            }
        }
        let kind = match op {
            ast::AssignOp::Set | ast::AssignOp::Draw => {
                let ty = if op == ast::AssignOp::Set {
                    self.declared_type(target)
                } else {
                    None
                };
                let mut v = self.expr_expected(value, ty.as_ref(), out);
                let mut hoisted = Vec::new();
                let Some(place) = self.place(target, &mut hoisted, "assign to") else {
                    return;
                };
                let later = self.later(&hoisted);
                self.stabilize(&mut v, later, out);
                out.extend(hoisted);
                if op == ast::AssignOp::Set {
                    StmtKind::Set { place, value: v }
                } else {
                    StmtKind::Draw { place, dist: v }
                }
            }
            ast::AssignOp::Add | ast::AssignOp::Sub | ast::AssignOp::Mul | ast::AssignOp::Div => {
                let bin = match op {
                    ast::AssignOp::Add => BinOp::Add,
                    ast::AssignOp::Sub => BinOp::Sub,
                    ast::AssignOp::Mul => BinOp::Mul,
                    _ => BinOp::Div,
                };
                let Some(mut place) = self.place(target, out, "assign to") else {
                    return;
                };
                let mut hoisted = Vec::new();
                let v = self.expr(value, &mut hoisted);
                // Read the place (and fix its indices) before the value's
                // statements run.
                let later = self.later(&hoisted);
                for elem in &mut place.path {
                    if let PathElem::Index(i) = elem {
                        self.stabilize(i, later, out);
                    }
                }
                let mut current = place_read(&place, target.span);
                self.stabilize(&mut current, later, out);
                out.extend(hoisted);
                StmtKind::Set {
                    place,
                    value: binary(bin, current, v, span),
                }
            }
        };
        let assigned = match &kind {
            StmtKind::Set { place, .. } | StmtKind::Draw { place, .. } => Some(place.slot),
            _ => None,
        };
        let st = self.stmt(span, kind);
        out.push(st);
        if let Some(slot_id) = assigned {
            let name = self.funcs[self.cur_func() as usize].slots[slot_id as usize]
                .name
                .clone();
            if let Some(ty) = self.lookup(&name).filter(|b| b.slot == slot_id).and_then(|b| b.ty) {
                let st = self.check(slot_id, &ty, span);
                out.push(st);
            }
        }
    }

    /// The type supplied by an annotation or a named record constructor.
    fn declared_type(&mut self, e: &ast::Expr) -> Option<TypeSpec> {
        match &e.kind {
            ast::ExprKind::Name(n) => self.lookup(n)?.ty,
            ast::ExprKind::Record { name: Some(n), .. } => {
                self.record_by_name.get(&n.name).copied().map(TypeSpec::Record)
            }
            ast::ExprKind::With { expr, .. } => self.declared_type(expr),
            ast::ExprKind::Field { expr, name } => match self.declared_type(expr)? {
                TypeSpec::Record(r) => self.records[r as usize]
                    .fields
                    .iter()
                    .find(|f| f.name == name.name)
                    .map(|f| f.ty.clone()),
                TypeSpec::AnonRecord(fields) => fields.into_iter().find(|(n, _)| n == &name.name).map(|(_, t)| t),
                _ => None,
            },
            ast::ExprKind::Index { expr, .. } => match self.declared_type(expr)? {
                TypeSpec::List(t) | TypeSpec::Map(_, t) => Some(*t),
                _ => None,
            },
            _ => None,
        }
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
                    .cloned();
                if let Some(b) = &local {
                    self.refer(e.span, b.def);
                }
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
                        if let Some(b) = self.lookup(name) {
                            self.refer(e.span, b.def);
                            self.error(e.span, format!("can't {action} `{name}` here"))
                                .note("functions, lambdas and `simulate` blocks can read outside variables but only change their own")
                                .help("return the new value instead");
                        } else if Constant::from_name(name).is_some() {
                            self.error(e.span, format!("can't {action} `{name}`: it's a constant"))
                                .help(format!("declare a variable of your own with `var {name} = …`"));
                        } else {
                            self.unknown_name(name, e.span);
                        }
                        None
                    }
                }
            }
            ast::ExprKind::Field { expr, name } => {
                let mut place = self.place(expr, out, action)?;
                self.field_uses.push((name.span, name.name.clone()));
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
                value: lit(Lit::Int((0).into()), span),
            },
        );
        out.push(set_items);
        out.push(set_index);

        let end = body.span.hi;
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
        // Integer ranges never repeat elements. For arbitrary collections,
        // aliases or nested loops, conservatively count visits. Compare slots,
        // not names: a binding inside the body may shadow the iteration key.
        let unique = matches!(
            &iter.kind,
            ast::ExprKind::Binary {
                op: BinOp::Range | BinOp::RangeExcl,
                ..
            }
        );
        let var = match &pattern.kind {
            ast::PatternKind::Name(name) if unique => self.lookup(name).map(|b| b.slot),
            _ => None,
        };
        self.cur().loops.push(LoopKind::For(var));
        let incr = self.stmt(
            span,
            StmtKind::Set {
                place: Place::slot(index),
                value: binary(BinOp::Add, slot(index, span), lit(Lit::Int((1).into()), span), span),
            },
        );
        inner.push(incr);
        let body = self.scoped_block(body);
        inner.extend(body.stmts);
        self.pop_scope(end);
        self.cur().loops.pop();
        let lp = self.stmt(
            span,
            StmtKind::Loop {
                body: Block { stmts: inner },
                bounded: true,
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
        if by.is_none() && !self.ctx[0].loops.is_empty() {
            self.error(span, "a `report` inside a loop needs `by`")
                .help("add a key that says which iteration each value belongs to, like `report x by month`");
            return;
        }
        let mut exprs = vec![value];
        exprs.extend(by);
        let mut values = self.operands(&exprs, out).into_iter();
        let v = values.next().unwrap();
        let key = values.next();
        let site = self.reports.len() as u32;
        let kind = match (self.ctx[0].loops.as_slice(), key.as_ref().map(|k| &k.kind)) {
            ([], _) => ReportKind::Once,
            ([LoopKind::For(Some(var))], Some(ExprKind::Slot(key))) if key == var => ReportKind::PerKey,
            _ => ReportKind::PerVisit,
        };
        self.reports.push(ReportSite {
            label: match label {
                Some((text, _)) => text.clone(),
                None => self.text(value.span),
            },
            key_label: by.map(|k| self.text(k.span)),
            kind,
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
        self.pop_scope(body.span.hi);
        Block { stmts }
    }

    fn block_into(&mut self, block: &ast::Block, dest: Option<SlotId>) -> Block {
        self.push_scope();
        let mut stmts = Vec::new();
        match dest {
            Some(d) => {
                let ty = self.funcs[self.cur_func() as usize].result_types.get(&d).cloned();
                let v = self.block_value_expected(block, ty.as_ref(), &mut stmts);
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
        self.pop_scope(block.span.hi);
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
        let c = self.expr_expected(cond, Some(&TypeSpec::Prob), out);
        let then = self.block_into(then, dest);
        let end = otherwise.map_or(span.hi, |o| o.span.hi);
        let otherwise = match otherwise.map(|o| &o.kind) {
            Some(ast::ExprKind::If { cond, then, otherwise }) => {
                let mut stmts = Vec::new();
                self.push_scope();
                self.if_into(cond, then, otherwise.as_deref(), dest, span, &mut stmts);
                self.pop_scope(end);
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
        let weights: Vec<&ast::Expr> = arms.iter().filter_map(|a| a.weight.as_ref()).collect();
        let types = vec![Some(TypeSpec::Prob); weights.len()];
        let mut weights = self.operands_expected(Vec::new(), &weights, &types, out).into_iter();
        for (i, arm) in arms.iter().enumerate() {
            match &arm.weight {
                Some(_) => {
                    let weight = weights.next().unwrap();
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
                value: builtin(Builtin::Settled, vec![v], scrutinee.span),
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
                self.pop_scope(arm.span.hi);
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
                value: lit(Lit::Bool(false), span),
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
                    value: lit(Lit::Bool(true), arm.span),
                },
            );
            let mut body = vec![mark];
            body.extend(self.branch_body(&arm.body, dest).stmts);
            match &arm.guard {
                Some(guard) => {
                    let g = self.expr_expected(guard, Some(&TypeSpec::Prob), &mut then);
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
            self.pop_scope(arm.span.hi);
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
            ast::PatternKind::Wildcard => lit(Lit::Bool(true), span),
            ast::PatternKind::Name(name) => {
                if let Some(variant) = self.variant(name, span) {
                    self.refer_variant(&variant, span);
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
                lit(Lit::Bool(true), span)
            }
            ast::PatternKind::Literal(e) => {
                let mut scratch = Vec::new();
                let l = self.expr(e, &mut scratch);
                binary(BinOp::Eq, value.clone(), l, span)
            }
            ast::PatternKind::List(items) => {
                let mut cond = builtin(
                    Builtin::IsListOfLen,
                    vec![value.clone(), lit(Lit::Int((items.len() as i64).into()), span)],
                    span,
                );
                for (i, item) in items.iter().enumerate() {
                    let element = Expr {
                        kind: ExprKind::Index(
                            Box::new(value.clone()),
                            Box::new(lit(Lit::Int((i as i64).into()), item.span)),
                        ),
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

    /// Lower operands left to right (docs/semantics.md, section 4). When an
    /// operand's lowering produces statements, the operands before it are
    /// first saved in temporaries, so they keep the values they had when they
    /// were evaluated.
    fn operands(&mut self, exprs: &[&ast::Expr], out: &mut Vec<Stmt>) -> Vec<Expr> {
        self.operands_after(Vec::new(), exprs, out)
    }

    /// Like `operands`, after some operands that are already lowered.
    fn operands_after(&mut self, values: Vec<Expr>, exprs: &[&ast::Expr], out: &mut Vec<Stmt>) -> Vec<Expr> {
        self.operands_expected(values, exprs, &[], out)
    }

    fn operands_expected(
        &mut self,
        mut values: Vec<Expr>,
        exprs: &[&ast::Expr],
        types: &[Option<TypeSpec>],
        out: &mut Vec<Stmt>,
    ) -> Vec<Expr> {
        for (v, ty) in values.iter_mut().zip(types) {
            if let Some(ty) = ty {
                self.contextualize(v, ty);
            }
        }
        for e in exprs {
            let mut hoisted = Vec::new();
            let v = self.expr_expected(e, types.get(values.len()).and_then(Option::as_ref), &mut hoisted);
            let later = self.later(&hoisted);
            for prev in values.iter_mut() {
                self.stabilize(prev, later, out);
            }
            out.extend(hoisted);
            values.push(v);
        }
        values
    }

    fn expr_expected(&mut self, e: &ast::Expr, ty: Option<&TypeSpec>, out: &mut Vec<Stmt>) -> Expr {
        let Some(ty) = ty else { return self.expr(e, out) };
        let kind = match (&e.kind, ty) {
            (ast::ExprKind::If { .. } | ast::ExprKind::Chance { .. } | ast::ExprKind::Match { .. }, _) => {
                let dest = self.temp(e.span);
                let func = self.cur_func() as usize;
                self.funcs[func].result_types.insert(dest, ty.clone());
                self.expr_into(e, dest, out);
                return slot(dest, e.span);
            }
            (ast::ExprKind::Block(block), _) => {
                self.push_scope();
                let v = self.block_value_expected(block, Some(ty), out);
                self.pop_scope(block.span.hi);
                return v;
            }
            (ast::ExprKind::List(xs), TypeSpec::List(t)) => {
                let refs: Vec<_> = xs.iter().collect();
                let types = vec![Some((**t).clone()); xs.len()];
                ExprKind::List(self.operands_expected(Vec::new(), &refs, &types, out))
            }
            (ast::ExprKind::Map(xs), TypeSpec::Map(_, v)) => {
                let refs: Vec<_> = xs.iter().flat_map(|(k, v)| [k, v]).collect();
                let types: Vec<_> = xs.iter().flat_map(|_| [None, Some((**v).clone())]).collect();
                let mut values = self.operands_expected(Vec::new(), &refs, &types, out).into_iter();
                let mut pairs = Vec::with_capacity(xs.len());
                while let (Some(k), Some(v)) = (values.next(), values.next()) {
                    pairs.push((k, v));
                }
                ExprKind::Map(pairs)
            }
            (ast::ExprKind::Record { name: None, fields }, TypeSpec::AnonRecord(types)) => {
                for field in fields {
                    self.anonymous_fields.insert(field.name.name.clone());
                }
                let types: Vec<_> = fields
                    .iter()
                    .map(|f| types.iter().find(|(n, _)| n == &f.name.name).map(|(_, t)| t.clone()))
                    .collect();
                ExprKind::Record {
                    ty: None,
                    fields: self.fields_expected(Vec::new(), fields, &types, out),
                }
            }
            _ => {
                let mut value = self.expr(e, out);
                self.contextualize(&mut value, ty);
                return value;
            }
        };
        Expr { kind, span: e.span }
    }

    fn later(&self, stmts: &[Stmt]) -> Later {
        Later {
            any: !stmts.is_empty(),
            assigns: self.assigns_variables(stmts),
        }
    }

    /// Whether statements may assign a program variable (not just
    /// temporaries). Calls can't: functions only assign their own variables.
    fn assigns_variables(&self, stmts: &[Stmt]) -> bool {
        let func = &self.funcs[self.cur_func() as usize];
        let is_temp = |slot: SlotId| func.slots[slot as usize].name == TEMP;
        fn any(stmts: &[Stmt], is_temp: &dyn Fn(SlotId) -> bool) -> bool {
            stmts.iter().any(|s| match &s.kind {
                StmtKind::Set { place, .. } | StmtKind::Draw { place, .. } | StmtKind::Take { place, .. } => {
                    !is_temp(place.slot)
                }
                StmtKind::If { then, otherwise, .. } => any(&then.stmts, is_temp) || any(&otherwise.stmts, is_temp),
                StmtKind::Chance { arms, otherwise, .. } => {
                    arms.iter().any(|(_, b)| any(&b.stmts, is_temp))
                        || otherwise.as_ref().is_some_and(|b| any(&b.stmts, is_temp))
                }
                StmtKind::Loop { body, .. } => any(&body.stmts, is_temp),
                _ => false,
            })
        }
        any(stmts, &is_temp)
    }

    /// Evaluate `e` into a temporary now, before the `later` statements,
    /// unless that can't make a difference. A variable needs it only if the
    /// statements assign variables. Any other computation needs it whenever
    /// there are statements: it can fail, print or run `simulate`, and the
    /// statements can split worlds, rule them out, or fail first.
    fn stabilize(&mut self, e: &mut Expr, later: Later, out: &mut Vec<Stmt>) {
        let stable = match &e.kind {
            ExprKind::Lit(_) => true,
            ExprKind::Slot(s) => !later.assigns || self.funcs[self.cur_func() as usize].slots[*s as usize].name == TEMP,
            _ => !later.any,
        };
        if stable {
            return;
        }
        let t = self.temp(e.span);
        let st = self.stmt(
            e.span,
            StmtKind::Set {
                place: Place::slot(t),
                value: e.clone(),
            },
        );
        out.push(st);
        *e = slot(t, e.span);
    }

    /// Lower `e` and store its value in `dest`.
    fn expr_into(&mut self, e: &ast::Expr, dest: SlotId, out: &mut Vec<Stmt>) {
        match &e.kind {
            ast::ExprKind::If { cond, then, otherwise } => {
                self.if_into(cond, then, otherwise.as_deref(), Some(dest), e.span, out)
            }
            ast::ExprKind::Chance { arms } => self.chance_into(arms, Some(dest), e.span, out),
            ast::ExprKind::Match { scrutinee, arms } => self.match_into(scrutinee, arms, Some(dest), e.span, out),
            _ => {
                let ty = self.funcs[self.cur_func() as usize].result_types.get(&dest).cloned();
                let v = self.expr_expected(e, ty.as_ref(), out);
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
            ast::ExprKind::Int(v) => ExprKind::Lit(Lit::Int(v.clone())),
            ast::ExprKind::Float(v) => ExprKind::Lit(Lit::Float(*v)),
            ast::ExprKind::Percent(v) => ExprKind::Lit(Lit::Float(*v)),
            ast::ExprKind::Dice { count, sides } => ExprKind::Lit(Lit::Dice {
                count: *count,
                sides: *sides,
            }),
            ast::ExprKind::Bool(b) => ExprKind::Lit(Lit::Bool(*b)),
            ast::ExprKind::Str(segments) => {
                let inner: Vec<&ast::Expr> = segments
                    .iter()
                    .filter_map(|seg| match seg {
                        ast::StrSegment::Expr(e) => Some(e),
                        ast::StrSegment::Lit(_) => None,
                    })
                    .collect();
                let mut values = self.operands(&inner, out).into_iter();
                let mut parts = Vec::new();
                for seg in segments {
                    match seg {
                        ast::StrSegment::Lit(text) => parts.push(InterpPart::Lit(text.clone())),
                        ast::StrSegment::Expr(_) => parts.push(InterpPart::Expr(values.next().unwrap())),
                    }
                }
                match parts.as_slice() {
                    [] => ExprKind::Lit(Lit::Str(String::new())),
                    [InterpPart::Lit(text)] => ExprKind::Lit(Lit::Str(text.clone())),
                    _ => ExprKind::Interp(parts),
                }
            }
            ast::ExprKind::Name(name) => return self.name(name, span),
            ast::ExprKind::Draw(source) => {
                let dest = self.temp(span);
                let kind = StmtKind::Draw {
                    place: Place::slot(dest),
                    dist: self.expr(source, out),
                };
                let st = self.stmt(span, kind);
                out.push(st);
                return slot(dest, span);
            }
            ast::ExprKind::List(items) => {
                let items: Vec<&ast::Expr> = items.iter().collect();
                ExprKind::List(self.operands(&items, out))
            }
            ast::ExprKind::Map(entries) => {
                let flat: Vec<&ast::Expr> = entries.iter().flat_map(|(k, v)| [k, v]).collect();
                let mut values = self.operands(&flat, out).into_iter();
                let mut pairs = Vec::with_capacity(entries.len());
                while let (Some(k), Some(v)) = (values.next(), values.next()) {
                    pairs.push((k, v));
                }
                ExprKind::Map(pairs)
            }
            ast::ExprKind::Record { name, fields } => self.record(name.as_ref(), fields, span, out),
            ast::ExprKind::Unary {
                op: ast::UnOp::Typeof,
                expr,
            } => {
                return builtin(Builtin::Typeof, vec![self.expr(expr, out)], span);
            }
            ast::ExprKind::Unary { op, expr } => ExprKind::Unary(*op, Box::new(self.expr(expr, out))),
            ast::ExprKind::Binary { op, lhs, rhs } => return self.binary(*op, lhs, rhs, span, out),
            ast::ExprKind::Call { callee, args } => return self.call(callee, args, span, out),
            ast::ExprKind::Method { receiver, name, args } => return self.method(receiver, name, args, span, out),
            ast::ExprKind::Field { expr, name } => {
                if let ast::ExprKind::Name(base) = &expr.kind {
                    if let Some(&ty) = self.enum_by_name.get(base) {
                        if self.lookup(base).is_none() {
                            if let Some(&def) = self.type_defs.get(base) {
                                self.refer(expr.span, def);
                            }
                            let enum_type = &self.enums[ty as usize];
                            return match enum_type.variants.iter().position(|v| *v == name.name) {
                                Some(i) => {
                                    let variant = Lit::Enum { ty, variant: i as u32 };
                                    self.refer_variant(&variant, name.span);
                                    lit(variant, span)
                                }
                                None => {
                                    let enum_name = enum_type.name.clone();
                                    self.error(name.span, format!("`{enum_name}` has no variant `{}`", name.name));
                                    lit(Lit::Unit, span)
                                }
                            };
                        }
                    }
                }
                self.field_uses.push((name.span, name.name.clone()));
                ExprKind::Field(Box::new(self.expr(expr, out)), name.name.clone())
            }
            ast::ExprKind::Index { expr, index } => {
                let mut values = self.operands(&[expr, index], out).into_iter();
                let (base, i) = (values.next().unwrap(), values.next().unwrap());
                ExprKind::Index(Box::new(base), Box::new(i))
            }
            ast::ExprKind::With { expr, fields } => {
                for field in fields {
                    self.field_uses.push((field.name.span, field.name.name.clone()));
                }
                let record = match self.declared_type(expr) {
                    Some(TypeSpec::Record(r)) => Some(r),
                    Some(TypeSpec::Dist(t)) => match *t {
                        TypeSpec::Record(r) => Some(r),
                        _ => None,
                    },
                    _ => None,
                };
                let types: Vec<_> = std::iter::once(None)
                    .chain(fields.iter().map(|f| {
                        record.and_then(|r| {
                            self.records[r as usize]
                                .fields
                                .iter()
                                .find(|d| d.name == f.name.name)
                                .map(|d| d.ty.clone())
                        })
                    }))
                    .collect();
                let base = self.expr(expr, out);
                let mut values = self.fields_expected(vec![base], fields, &types, out);
                let base = values.remove(0).1;
                ExprKind::With(Box::new(base), values)
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
                self.pop_scope(block.span.hi);
                return v;
            }
        };
        Expr { kind, span }
    }

    fn name(&mut self, name: &str, span: Span) -> Expr {
        if let Some(b) = self.lookup(name) {
            self.refer(span, b.def);
            return slot(b.slot, span);
        }
        if let Some(variant) = self.variant(name, span) {
            self.refer_variant(&variant, span);
            return lit(variant, span);
        }
        if let Some(c) = Constant::from_name(name).filter(|_| !self.fn_by_name.contains_key(name)) {
            return match c.value() {
                Some(value) => lit(Lit::FloatConstant(value), span),
                None => builtin(Builtin::RunDate, vec![], span),
            };
        }
        if let Some(&func) = self.fn_by_name.get(name) {
            if let Some(&def) = self.fn_defs.get(name) {
                self.refer(span, def);
            }
            let caller = self.cur_func();
            self.funcs[caller as usize].calls.insert(func);
            return Expr {
                kind: ExprKind::Closure {
                    func,
                    capture_args: Vec::new(),
                },
                span,
            };
        }
        if let Some(b) = Builtin::from_name(name) {
            return lit(Lit::Builtin(b), span);
        }
        if self.record_by_name.contains_key(name) || self.enum_by_name.contains_key(name) {
            self.error(span, format!("the type `{name}` can't be used as a value"));
        } else {
            self.unknown_name(name, span);
        }
        lit(Lit::Unit, span)
    }

    /// Lower record fields in order, after already-lowered operands (which
    /// come back first in the result, with empty names).
    fn fields_expected(
        &mut self,
        before: Vec<Expr>,
        fields: &[ast::Field],
        types: &[Option<TypeSpec>],
        out: &mut Vec<Stmt>,
    ) -> Vec<(String, Expr)> {
        let mut names: Vec<String> = Vec::new();
        let mut exprs: Vec<&ast::Expr> = Vec::new();
        for field in fields {
            if names.contains(&field.name.name) {
                self.error(
                    field.name.span,
                    format!("the field `{}` appears twice", field.name.name),
                );
                continue;
            }
            names.push(field.name.name.clone());
            exprs.push(&field.value);
        }
        let n_before = before.len();
        let values = self.operands_expected(before, &exprs, types, out);
        let mut result: Vec<(String, Expr)> = Vec::with_capacity(values.len());
        for (i, v) in values.into_iter().enumerate() {
            let name = if i < n_before {
                String::new()
            } else {
                names[i - n_before].clone()
            };
            result.push((name, v));
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
        let types = name
            .and_then(|n| self.record_by_name.get(&n.name))
            .map(|id| {
                fields
                    .iter()
                    .map(|f| {
                        self.records[*id as usize]
                            .fields
                            .iter()
                            .find(|d| d.name == f.name.name)
                            .map(|d| d.ty.clone())
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let lowered = self.fields_expected(Vec::new(), fields, &types, out);
        let Some(name) = name else {
            for field in fields {
                self.anonymous_fields.insert(field.name.name.clone());
            }
            return ExprKind::Record {
                ty: None,
                fields: lowered,
            };
        };
        let Some(&ty) = self.record_by_name.get(&name.name) else {
            self.error(name.span, format!("unknown record type `{}`", name.name));
            return ExprKind::Lit(Lit::Unit);
        };
        if let Some(&def) = self.type_defs.get(&name.name) {
            self.refer(name.span, def);
        }
        for field in fields {
            let def = self
                .field_defs
                .get(&field.name.name)
                .and_then(|defs| defs.iter().find(|&&(r, _)| r == ty));
            if let Some(&(_, def)) = def {
                self.refer(field.name.span, def);
            }
        }
        let declared: Vec<String> = self.records[ty as usize]
            .fields
            .iter()
            .map(|f| f.name.clone())
            .collect();
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
        let mut values = self.operands_after(vec![l], &[rhs], out).into_iter();
        let (l, r) = (values.next().unwrap(), values.next().unwrap());
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
        let named = self.argument_names(args);
        let exprs: Vec<_> = args.iter().map(|a| &a.value).collect();
        let mut values = self.operands_after(vec![f], &exprs, out);
        let f = values.remove(0);
        self.hoist_call(Callee::Value(f, named), values, span, out)
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
                    if !args.is_empty() {
                        self.error(span, "`take()` doesn't take arguments");
                    }
                    let Some(bag) = self.place(receiver, out, "take an item from") else {
                        return lit(Lit::Unit, span);
                    };
                    let dest = self.temp(span);
                    let st = self.stmt(
                        span,
                        StmtKind::Take {
                            place: Place::slot(dest),
                            bag,
                        },
                    );
                    out.push(st);
                    return slot(dest, span);
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
        let receiver_type = self.declared_type(receiver);
        let types = match (&receiver_type, b) {
            (Some(TypeSpec::List(t)), Builtin::Push) => vec![None, Some((**t).clone())],
            (Some(TypeSpec::List(t)), Builtin::Insert) => vec![None, None, Some((**t).clone())],
            (Some(TypeSpec::Map(k, v)), Builtin::Insert) => vec![None, Some((**k).clone()), Some((**v).clone())],
            _ => Vec::new(),
        };
        let all = self.positional_expected(vec![place_read(&place, receiver.span)], args, &types, out);
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
                place: place.clone(),
                value: builtin(b, all, span),
            },
        );
        out.push(st);
        let root_name = self.funcs[self.cur_func() as usize].slots[place.slot as usize]
            .name
            .clone();
        if let Some(ty) = self
            .lookup(&root_name)
            .filter(|b| b.slot == place.slot)
            .and_then(|b| b.ty)
        {
            let check = self.check(place.slot, &ty, span);
            out.push(check);
        }
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
            let ty = if let Some(&func) = self.fn_by_name.get(&name.name) {
                self.funcs[func as usize].param_types.first().cloned().flatten()
            } else {
                Builtin::from_name(&name.name)
                    .filter(|b| b.probability_parameter(0))
                    .map(|_| TypeSpec::Prob)
            };
            values.push(self.expr_expected(r, ty.as_ref(), out));
        }
        if let Some(&func) = self.fn_by_name.get(&name.name) {
            if let Some(&def) = self.fn_defs.get(&name.name) {
                self.refer(name.span, def);
            }
            let types = self.funcs[func as usize].param_types.clone();
            let values = self.positional_expected(values, args, &types, out);
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
        if name.name == "read" {
            self.error(
                span,
                "`read` must be the whole value of a `let` with a type, at the top level",
            )
            .help("like `let rows: list[Row] = read(\"rows.csv\")`: the data is read before the program runs");
            return lit(Lit::Unit, span);
        }
        if let Some(b) = Builtin::from_name(&name.name) {
            if b == Builtin::Take {
                self.error(span, "`take` needs a mutable bag receiver")
                    .help("write `deck.take()` to draw and remove an item from `deck`");
                return lit(Lit::Unit, span);
            }
            let names = self.argument_names(args);
            for key in &names {
                if !matches!(b, Builtin::Minimum | Builtin::Maximum) || key != "default" {
                    self.error(span, format!("`{}` doesn't take the named argument `{key}`", name.name));
                }
            }
            let positional_count = values.len() + args.len() - names.len();
            if !names.is_empty() && positional_count == 0 {
                self.error(span, "a collection argument is required before `default`");
            }
            if !names.is_empty() && positional_count >= 3 {
                self.error(span, "the default was supplied both positionally and by name");
            }
            let exprs: Vec<&ast::Expr> = args.iter().map(|a| &a.value).collect();
            let types: Vec<_> = (0..values.len() + exprs.len())
                .map(|i| b.probability_parameter(i).then_some(TypeSpec::Prob))
                .collect();
            let mut values = self.operands_expected(values, &exprs, &types, out);
            self.check_arity(b, values.len(), span);
            let named_values = values.split_off(positional_count);
            let named = names.into_iter().zip(named_values).collect();
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
        } else if Constant::from_name(&name.name).is_some() {
            self.error(name.span, format!("`{}` is a constant, not a function", name.name))
                .help(format!("use `{}` without parentheses", name.name));
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
        if n < min || n > max || (b == Builtin::Date && n == 2) {
            let expected = if b == Builtin::Date {
                "one ISO string or three integers (year, month, day)".to_string()
            } else if min == max {
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

    /// Named arguments follow positional arguments, preserving evaluation order.
    fn argument_names(&mut self, args: &[ast::Arg]) -> Vec<String> {
        let mut names = Vec::new();
        for arg in args {
            if let Some(name) = &arg.name {
                if names.contains(&name.name) {
                    self.error(name.span, format!("the named argument `{}` appears twice", name.name));
                }
                names.push(name.name.clone());
            } else if !names.is_empty() {
                self.error(arg.value.span, "positional arguments must come before named arguments");
            }
        }
        names
    }

    fn positional_expected(
        &mut self,
        before: Vec<Expr>,
        args: &[ast::Arg],
        types: &[Option<TypeSpec>],
        out: &mut Vec<Stmt>,
    ) -> Vec<Expr> {
        for arg in args {
            if let Some(n) = &arg.name {
                self.error(n.span, "only built-in functions take named arguments");
            }
        }
        let exprs: Vec<&ast::Expr> = args.iter().map(|a| &a.value).collect();
        self.operands_expected(before, &exprs, types, out)
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
            loops: Vec::new(),
            ret: None,
        });
        for p in params {
            let (_, def) = self.declare_as(&p.name, p.span, false, DefKind::Parameter);
            self.symbols.definitions[def].scope.hi = body.span.hi;
        }
        self.funcs[id as usize].n_params = params.len() as u32;
        let mut stmts = Vec::new();
        self.push_scope();
        let v = self.expr(body, &mut stmts);
        self.pop_scope(body.span.hi);
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
            loops: Vec::new(),
            ret: None,
        });
        let mut stmts = Vec::new();
        self.push_scope();
        let v = self.block_value(block, &mut stmts);
        self.pop_scope(block.span.hi);
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
                effects: Effects::default(),
            })
            .collect();
        let mut program = Program {
            functions,
            reports: self.reports,
            records: self.records,
            enums: self.enums,
            inputs: self.inputs,
            settings: self.settings,
            stmt_count: self.next_stmt,
        };
        let mut diags = self.diags;
        diags.extend(crate::effects::analyze(&mut program, self.src));
        crate::draws::move_draws(&mut program);
        (program, diags)
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
                Callee::Value(e, _) => visit_expr(e, f),
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
        StmtKind::Loop { body, .. } => visit_block(body, f),
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
        StmtKind::Break | StmtKind::Continue | StmtKind::Fail { .. } | StmtKind::Check { .. } => {}
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
        ExprKind::Lit(_) | ExprKind::Slot(_) | ExprKind::Input(_) => {}
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

fn type_span(t: &ast::TypeExpr) -> Span {
    match t {
        ast::TypeExpr::Named { name, args } => match args.last() {
            Some(last) => name.span.to(type_span(last)),
            None => name.span,
        },
        ast::TypeExpr::Record { span, .. } => *span,
    }
}

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
        ast::ExprKind::Int(ref v) => v.to_u64(),
        _ => None,
    }
}

fn pragma_float(e: &ast::Expr) -> Option<f64> {
    match e.kind {
        ast::ExprKind::Int(ref v) => v.to_f64(),
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
