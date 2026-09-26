//! The intermediate representation run by the engine.
//!
//! Control flow stays structured (blocks, `if`, `chance`, loops). The key
//! property is that **expressions never split worlds**: everything that can
//! (draws, calls to user functions, `if`/`chance`/`match` used as values) has
//! been moved into a statement of its own by the lowering pass. Expressions
//! can therefore be evaluated independently in each world.

use crate::builtins::Builtin;
use probl_syntax::Span;
use probl_syntax::ast::{BinOp, UnOp};

pub type SlotId = u32;
pub type FnId = u32;
pub type StmtId = u32;

/// The function holding the top-level statements.
pub const MAIN: FnId = 0;

#[derive(Debug)]
pub struct Program {
    /// `functions[MAIN]` is the top level; its slots include the globals.
    pub functions: Vec<Function>,
    pub reports: Vec<ReportSite>,
    pub records: Vec<RecordType>,
    pub enums: Vec<EnumType>,
    pub settings: Settings,
    /// Number of statements; statement ids are `0..stmt_count`.
    pub stmt_count: u32,
}

impl Program {
    pub fn main(&self) -> &Function {
        &self.functions[MAIN as usize]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub mode: Mode,
    /// Where `@mode` was set, for error messages.
    pub mode_span: Option<Span>,
    /// Loops stop once the worlds still inside weigh less than this.
    pub epsilon: f64,
    pub max_iterations: u64,
    pub max_worlds: usize,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            mode: Mode::Auto,
            mode_span: None,
            epsilon: 1e-12,
            max_iterations: 10_000_000,
            max_worlds: 10_000_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    Auto,
    Exact,
    Beam { worlds: u64 },
    Sample { runs: u64, seed: u64 },
    Particles { runs: u64, seed: u64 },
}

impl Mode {
    pub fn name(&self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Exact => "exact",
            Mode::Beam { .. } => "beam",
            Mode::Sample { .. } => "sample",
            Mode::Particles { .. } => "particles",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReportSite {
    pub label: String,
    /// Source text of the `by` expression.
    pub key_label: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct RecordType {
    pub name: String,
    /// In declaration order.
    pub fields: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EnumType {
    pub name: String,
    pub variants: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FnKind {
    Main,
    Named,
    Lambda,
    Simulate,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub kind: FnKind,
    pub span: Span,
    /// Parameters occupy slots `0..n_params`.
    pub n_params: u32,
    /// Values copied into the frame on entry, after the parameters.
    pub captures: Vec<Capture>,
    pub slots: Vec<SlotInfo>,
    pub body: Block,
}

impl Function {
    pub fn n_slots(&self) -> usize {
        self.slots.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capture {
    /// Where the value goes in this function's frame.
    pub slot: SlotId,
    pub source: CaptureSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureSource {
    /// A top-level variable (a slot of the main frame).
    Global(SlotId),
    /// A variable of the enclosing function (for lambdas and `simulate`).
    Parent(SlotId),
}

#[derive(Clone, Debug)]
pub struct SlotInfo {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, Default)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub id: StmtId,
    pub span: Span,
    pub kind: StmtKind,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    /// `place = value`.
    Set {
        place: Place,
        value: Expr,
    },
    /// `place ~ dist`: one world per outcome.
    Draw {
        place: Place,
        dist: Expr,
    },
    /// `place ~ bag.take()`: draw a card and remove it from the bag.
    Take {
        place: Place,
        bag: Place,
    },
    /// Call a user function or closure; may split worlds.
    Call {
        dest: Place,
        callee: Callee,
        args: Vec<Expr>,
    },
    If {
        cond: Expr,
        then: Block,
        otherwise: Block,
    },
    /// Weighted branches. Without `otherwise`, the remaining weight either
    /// continues past the statement (`exhaustive: false`) or is an error.
    Chance {
        arms: Vec<(Expr, Block)>,
        otherwise: Option<Block>,
        exhaustive: bool,
    },
    Loop {
        body: Block,
    },
    Break,
    Continue,
    Return(Expr),
    Observe {
        value: Expr,
        from: Option<Expr>,
    },
    Report {
        site: u32,
        value: Expr,
        key: Option<Expr>,
    },
    /// A runtime error in every world that reaches it.
    Fail {
        message: String,
    },
}

/// A variable, optionally followed by fields and indices: `a.b[i]`.
#[derive(Clone, Debug)]
pub struct Place {
    pub slot: SlotId,
    pub path: Vec<PathElem>,
}

impl Place {
    pub fn slot(slot: SlotId) -> Place {
        Place { slot, path: Vec::new() }
    }
}

#[derive(Clone, Debug)]
pub enum PathElem {
    Field(String),
    Index(Expr),
}

#[derive(Clone, Debug)]
pub enum Callee {
    /// A named function; `capture_args[i]` is the caller's slot supplying the
    /// callee's `captures[i]`.
    Fn { func: FnId, capture_args: Vec<SlotId> },
    /// A closure value.
    Value(Expr),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Lit(Lit),
    Slot(SlotId),
    Unary(UnOp, Box<Expr>),
    /// All binary operators, including `and` and `or` (whose right side never
    /// contains hoisted statements, so it can be evaluated lazily).
    Binary(BinOp, Box<Expr>, Box<Expr>),
    List(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    Record {
        ty: Option<u32>,
        fields: Vec<(String, Expr)>,
    },
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    With(Box<Expr>, Vec<(String, Expr)>),
    Builtin {
        func: Builtin,
        args: Vec<Expr>,
        named: Vec<(String, Expr)>,
    },
    /// Create a closure; `capture_args` align with the lambda's captures.
    Closure {
        func: FnId,
        capture_args: Vec<SlotId>,
    },
    /// Run a `simulate` block and return its distribution.
    Simulate {
        func: FnId,
        capture_args: Vec<SlotId>,
    },
    Interp(Vec<InterpPart>),
}

#[derive(Clone, Debug)]
pub enum InterpPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    Unit,
    Int(i64),
    Float(f64),
    Prob(f64),
    Str(String),
    Dice { count: u32, sides: u32 },
    Enum { ty: u32, variant: u32 },
}
