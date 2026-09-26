//! The abstract syntax tree produced by the parser.

use crate::span::Span;

#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub pragmas: Vec<Pragma>,
    pub items: Vec<Item>,
}

/// `@mode exact`, `@mode sample(runs: 1000)`, `@epsilon 1e-9`.
#[derive(Clone, Debug, PartialEq)]
pub struct Pragma {
    pub name: Ident,
    pub arg: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Fn(FnDecl),
    Type(TypeDecl),
    Enum(EnumDecl),
    Import(Import),
    Stmt(Stmt),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FnDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: Ident,
    pub ty: Option<TypeExpr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnumDecl {
    pub name: Ident,
    pub variants: Vec<Ident>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub path: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeExpr {
    /// `int`, `list[int]`, `dist[int]`, `Fighter`.
    Named { name: Ident, args: Vec<TypeExpr> },
    /// `{ hp: int, ac: int }`.
    Record { fields: Vec<(Ident, TypeExpr)>, span: Span },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

/// `=` or `~` in a binding or assignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindOp {
    Assign,
    Draw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Draw,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    /// `let` / `var` bindings.
    Let {
        mutable: bool,
        pattern: Pattern,
        ty: Option<TypeExpr>,
        op: BindOp,
        value: Expr,
    },
    Assign {
        target: Expr,
        op: AssignOp,
        value: Expr,
    },
    For {
        pattern: Pattern,
        iter: Expr,
        body: Block,
    },
    While {
        cond: Expr,
        body: Block,
    },
    Repeat {
        count: Expr,
        body: Block,
    },
    Loop {
        body: Block,
    },
    Break,
    Continue,
    Return(Option<Expr>),
    Observe {
        value: Expr,
        from: Option<Expr>,
    },
    Report {
        value: Expr,
        by: Option<Expr>,
        label: Option<(String, Span)>,
    },
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    /// Already divided by 100.
    Percent(f64),
    Dice {
        count: u32,
        sides: u32,
    },
    Str(Vec<StrSegment>),
    Bool(bool),
    Name(String),
    List(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    /// `Fighter { hp: 12 }` (named) or `{ won: true }` (anonymous).
    Record {
        name: Option<Ident>,
        fields: Vec<Field>,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Arg>,
    },
    /// `receiver.name(args)`.
    Method {
        receiver: Box<Expr>,
        name: Ident,
        args: Vec<Arg>,
    },
    Field {
        expr: Box<Expr>,
        name: Ident,
    },
    Index {
        expr: Box<Expr>,
        index: Box<Expr>,
    },
    /// `expr with { field: value }`.
    With {
        expr: Box<Expr>,
        fields: Vec<Field>,
    },
    Lambda {
        params: Vec<Ident>,
        body: Box<Expr>,
    },
    If {
        cond: Box<Expr>,
        then: Block,
        otherwise: Option<Box<Expr>>,
    },
    Chance {
        arms: Vec<ChanceArm>,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    Simulate(Block),
    Block(Block),
}

#[derive(Clone, Debug, PartialEq)]
pub enum StrSegment {
    Lit(String),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: Ident,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arg {
    pub name: Option<Ident>,
    pub value: Expr,
}

/// `weight => body`; the weight is `None` for `else`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChanceArm {
    pub weight: Option<Expr>,
    pub body: Stmt,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Stmt,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    NotIn,
    /// `a..b`, both ends included.
    Range,
    /// `a..<b`.
    RangeExcl,
    /// `a to b`: an estimate with a 90% interval.
    To,
    Add,
    Sub,
    Mul,
    Div,
    IntDiv,
    Mod,
    Pow,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Or => "or",
            BinOp::And => "and",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::In => "in",
            BinOp::NotIn => "not in",
            BinOp::Range => "..",
            BinOp::RangeExcl => "..<",
            BinOp::To => "to",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::IntDiv => "div",
            BinOp::Mod => "mod",
            BinOp::Pow => "^",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PatternKind {
    Wildcard,
    /// A name: binds a new variable, or matches an enum variant of that name.
    Name(String),
    Literal(Expr),
    List(Vec<Pattern>),
    Or(Vec<Pattern>),
}
