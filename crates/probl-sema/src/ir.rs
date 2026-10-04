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

/// The name of compiler-generated variables. Each is assigned once per path
/// and never changed afterwards.
pub const TEMP: &str = "(temporary)";

#[derive(Debug)]
pub struct Program {
    /// `functions[MAIN]` is the top level; its slots include the globals.
    pub functions: Vec<Function>,
    pub reports: Vec<ReportSite>,
    pub records: Vec<RecordType>,
    pub enums: Vec<EnumType>,
    /// The data the program reads: whoever runs it loads these first.
    pub inputs: Vec<Input>,
    pub settings: Settings,
    /// Number of statements; statement ids are `0..stmt_count`.
    pub stmt_count: u32,
}

/// Data a program reads, `let name: T = read("path")` (docs/data-input.md).
/// The program's list of these is its manifest: whoever runs the program
/// loads each input before running it, and the engine only sees the values.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    /// The variable it's bound to, which identifies it.
    pub name: String,
    /// As written: relative to the program's file, or `-` for standard
    /// input. What it means is up to whoever loads it.
    pub path: String,
    pub format: DataFormat,
    /// What the data is read as.
    pub ty: TypeSpec,
    /// The `read(…)` call.
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataFormat {
    Csv,
    Json,
    Lines,
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
    /// Unbounded loops stop once the weight still inside is less than this
    /// fraction of the weight that entered.
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
    Enumerate,
    Beam { worlds: u64 },
    Sample { runs: u64, seed: u64 },
    Particles { runs: u64, seed: u64 },
}

impl Mode {
    pub fn name(&self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Enumerate => "enumerate",
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
    pub kind: ReportKind,
    pub span: Span,
}

/// How often one world can reach a report (see docs/semantics.md, section 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    /// Outside loops: at most once per world.
    Once,
    /// In a single loop with proven unique iteration keys: at most once per
    /// world and key, using the loop binding itself.
    PerKey,
    /// In a loop with any other key: every visit counts.
    PerVisit,
}

#[derive(Clone, Debug)]
pub struct RecordType {
    pub name: String,
    /// In declaration order.
    pub fields: Vec<RecordField>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct RecordField {
    pub name: String,
    pub ty: TypeSpec,
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
    pub effects: Effects,
}

/// What running a function can do besides computing its result, including
/// through the functions it calls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// Runs `observe` (outside any `simulate` block).
    pub observes: bool,
    /// Calls `print`. Such functions aren't memoized.
    pub prints: bool,
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
    /// `place = bag.take()`: select an item unchanged and remove one copy.
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
    /// Repeats until every world has left through `break` or `return`.
    /// Unbounded loops (`while`, `loop`) stop early once the weight still
    /// inside is negligible; bounded ones (`for`, `repeat`) never do.
    Loop {
        body: Block,
        bounded: bool,
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
    /// Check that a variable's value matches its declared type.
    Check {
        slot: SlotId,
        ty: TypeSpec,
    },
}

/// A declared type, checked when a value is stored.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeSpec {
    Int,
    Float,
    Complex,
    Prob,
    Bool,
    Str,
    Date,
    Unit,
    Function,
    List(Box<TypeSpec>),
    Map(Box<TypeSpec>, Box<TypeSpec>),
    Bag(Box<TypeSpec>),
    Dist(Box<TypeSpec>),
    Record(u32),
    Enum(u32),
    AnonRecord(Vec<(String, TypeSpec)>),
}

impl TypeSpec {
    /// How the type is written in source.
    pub fn describe(&self, program: &Program) -> String {
        self.describe_in(&program.records, &program.enums)
    }

    /// How the type is written in source, given the program's types.
    pub fn describe_in(&self, records: &[RecordType], enums: &[EnumType]) -> String {
        let describe = |t: &TypeSpec| t.describe_in(records, enums);
        match self {
            TypeSpec::Int => "int".into(),
            TypeSpec::Float => "float".into(),
            TypeSpec::Complex => "complex".into(),
            TypeSpec::Prob => "prob".into(),
            TypeSpec::Bool => "bool".into(),
            TypeSpec::Str => "str".into(),
            TypeSpec::Date => "date".into(),
            TypeSpec::Unit => "()".into(),
            TypeSpec::Function => "fn".into(),
            TypeSpec::List(t) => format!("list[{}]", describe(t)),
            TypeSpec::Map(k, v) => format!("map[{}, {}]", describe(k), describe(v)),
            TypeSpec::Bag(t) => format!("bag[{}]", describe(t)),
            TypeSpec::Dist(t) => format!("dist[{}]", describe(t)),
            TypeSpec::Record(r) => records[*r as usize].name.clone(),
            TypeSpec::Enum(e) => enums[*e as usize].name.clone(),
            TypeSpec::AnonRecord(fields) => {
                let fields: Vec<String> = fields.iter().map(|(n, t)| format!("{n}: {}", describe(t))).collect();
                format!("{{ {} }}", fields.join(", "))
            }
        }
    }
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
    /// A function value; names correspond to the final call arguments.
    Value(Expr, Vec<String>),
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
    /// The value of the program's input with this number (`read`).
    Input(u32),
}

#[derive(Clone, Debug)]
pub enum InterpPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    Builtin(Builtin),
    Unit,
    Bool(bool),
    Int(probl_number::Integer),
    Float(f64),
    /// A named mathematical constant: numeric, but not a source literal
    /// eligible for contextual conversion to `prob`.
    FloatConstant(f64),
    Prob(f64),
    Str(String),
    Dice {
        count: u32,
        sides: u32,
    },
    Enum {
        ty: u32,
        variant: u32,
    },
}
