//! Recursive-descent parser, with Pratt parsing for binary operators.

use crate::ast::*;
use crate::diagnostic::Diagnostic;
use crate::lexer::lex;
use crate::span::Span;
use crate::token::{StrPart, Tok, Token};

/// Parse a whole program. Parsing continues after errors, so the program is
/// returned even when there are diagnostics.
pub fn parse_program(src: &str) -> (Program, Vec<Diagnostic>) {
    let (tokens, mut diags) = lex(src, 0);
    let mut parser = Parser::new(tokens);
    let program = parser.program();
    diags.append(&mut parser.diags);
    diags.sort_by_key(|d| d.span.lo);
    (program, diags)
}

/// Parse a single expression (used for string interpolation and the REPL).
pub fn parse_expr(src: &str, base: u32) -> (Option<Expr>, Vec<Diagnostic>) {
    parse_nested_expr(src, base, 0)
}

/// How deeply expressions, blocks and patterns may nest. Deeper programs
/// would risk overflowing the stack of the recursive parser and of the passes
/// after it.
pub const MAX_NESTING: u32 = 200;

fn parse_nested_expr(src: &str, base: u32, depth: u32) -> (Option<Expr>, Vec<Diagnostic>) {
    let (tokens, mut diags) = lex(src, base);
    let mut parser = Parser::new(tokens);
    parser.depth = depth;
    let expr = parser.expr().ok();
    if expr.is_some() && !parser.at(&Tok::Eof) {
        let tok = parser.peek().clone();
        parser.error(
            parser.span(),
            format!("unexpected {} after the expression", tok.describe()),
        );
    }
    diags.append(&mut parser.diags);
    (expr, diags)
}

/// Signals that a diagnostic was recorded and the caller should recover.
#[derive(Debug)]
pub struct Failed;

type PResult<T> = Result<T, Failed>;

// Binding powers, from loosest to tightest.
const PREC_OR: u8 = 1;
const PREC_AND: u8 = 2;
const PREC_NOT: u8 = 3;
const PREC_CMP: u8 = 4;
const PREC_RANGE: u8 = 5;
const PREC_ADD: u8 = 6;
const PREC_MUL: u8 = 7;
const PREC_NEG: u8 = 8;
const PREC_POW: u8 = 9;
const PREC_TYPEOF: u8 = 10;

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    diags: Vec<Diagnostic>,
    /// Current nesting of expressions, blocks and patterns.
    depth: u32,
    /// Set while parsing the header of `if`, `while`, `for`, `repeat` and
    /// `match`, where `{` starts the body rather than a record.
    restricted: bool,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Parser {
        Parser {
            tokens,
            pos: 0,
            diags: Vec::new(),
            depth: 0,
            restricted: false,
        }
    }

    // ── Token helpers ────────────────────────────────────────────────────

    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].tok
    }

    fn peek_at(&self, ahead: usize) -> &Tok {
        let i = (self.pos + ahead).min(self.tokens.len() - 1);
        &self.tokens[i].tok
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn prev_span(&self) -> Span {
        if self.pos == 0 {
            self.span()
        } else {
            self.tokens[self.pos - 1].span
        }
    }

    fn at(&self, tok: &Tok) -> bool {
        self.peek() == tok
    }

    fn bump(&mut self) -> Token {
        let token = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        token
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.at(tok) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) -> &mut Diagnostic {
        self.diags.push(Diagnostic::error(span, message));
        self.diags.last_mut().unwrap()
    }

    fn expect(&mut self, tok: &Tok, context: &str) -> PResult<Span> {
        if self.at(tok) {
            Ok(self.bump().span)
        } else {
            let found = self.peek().describe();
            let span = self.span();
            self.error(span, format!("expected `{}` {context}, found {found}", tok.text()));
            Err(Failed)
        }
    }

    fn ident(&mut self, what: &str) -> PResult<Ident> {
        match self.peek().clone() {
            Tok::Ident(name) => {
                let span = self.bump().span;
                Ok(Ident { name, span })
            }
            other => {
                let span = self.span();
                self.error(span, format!("expected {what}, found {}", other.describe()));
                Err(Failed)
            }
        }
    }

    fn skip_newlines(&mut self) {
        while self.at(&Tok::Newline) {
            self.bump();
        }
    }

    fn skip_separators(&mut self) {
        while matches!(self.peek(), Tok::Newline | Tok::Semi) {
            self.bump();
        }
    }

    /// After a statement: a line break or `;`, or the end of the enclosing block.
    fn end_of_statement(&mut self) -> PResult<()> {
        match self.peek() {
            Tok::Newline | Tok::Semi => {
                self.bump();
                Ok(())
            }
            Tok::RBrace | Tok::Eof => Ok(()),
            other => {
                let found = other.describe();
                let span = self.span();
                self.error(span, format!("expected the end of the statement, found {found}"))
                    .help("put each statement on its own line, or separate them with `;`");
                Err(Failed)
            }
        }
    }

    /// Skip to the end of the current statement after an error.
    fn recover(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBracket => depth = depth.saturating_sub(1),
                Tok::RBrace => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                Tok::Newline | Tok::Semi if depth == 0 => {
                    self.bump();
                    return;
                }
                _ => {}
            }
            self.bump();
        }
    }

    /// Run `f` one level deeper, or fail if the program nests too deeply.
    fn nested<T>(&mut self, f: impl FnOnce(&mut Parser) -> PResult<T>) -> PResult<T> {
        if self.depth >= MAX_NESTING {
            let span = self.span();
            self.error(span, "this is nested too deeply").help(format!(
                "expressions, blocks and patterns can nest at most {MAX_NESTING} levels"
            ));
            return Err(Failed);
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        result
    }

    fn with_restriction<T>(&mut self, restricted: bool, f: impl FnOnce(&mut Parser) -> T) -> T {
        let saved = std::mem::replace(&mut self.restricted, restricted);
        let result = f(self);
        self.restricted = saved;
        result
    }

    // ── Program and items ────────────────────────────────────────────────

    fn program(&mut self) -> Program {
        let mut pragmas = Vec::new();
        let mut items = Vec::new();
        self.skip_separators();
        while self.at(&Tok::At) {
            match self.pragma() {
                Ok(p) => {
                    pragmas.push(p);
                    if self.end_of_statement().is_err() {
                        self.recover();
                    }
                }
                Err(Failed) => self.recover(),
            }
            self.skip_separators();
        }
        while !self.at(&Tok::Eof) {
            if self.at(&Tok::RBrace) {
                let span = self.span();
                self.error(span, "unexpected `}`")
                    .help("there is no open `{` for it to close");
                self.bump();
                self.skip_separators();
                continue;
            }
            match self.item() {
                Ok(item) => {
                    items.push(item);
                    if self.end_of_statement().is_err() {
                        self.recover();
                    }
                }
                Err(Failed) => self.recover(),
            }
            self.skip_separators();
        }
        Program { pragmas, items }
    }

    fn pragma(&mut self) -> PResult<Pragma> {
        let lo = self.expect(&Tok::At, "")?;
        let name = self.ident("a pragma name after `@`")?;
        let arg = if matches!(self.peek(), Tok::Newline | Tok::Semi | Tok::Eof) {
            None
        } else {
            Some(self.expr()?)
        };
        let span = lo.to(self.prev_span());
        Ok(Pragma { name, arg, span })
    }

    fn item(&mut self) -> PResult<Item> {
        match self.peek() {
            Tok::Fn => self.fn_decl().map(Item::Fn),
            Tok::Type => self.type_decl().map(Item::Type),
            Tok::Enum => self.enum_decl().map(Item::Enum),
            Tok::Import => {
                let lo = self.bump().span;
                match self.peek().clone() {
                    Tok::Str(parts) => {
                        let span = self.bump().span;
                        match plain_string(&parts) {
                            Some(path) => Ok(Item::Import(Import {
                                path,
                                span: lo.to(span),
                            })),
                            None => {
                                self.error(span, "an import path can't contain `{…}`");
                                Err(Failed)
                            }
                        }
                    }
                    other => {
                        let span = self.span();
                        self.error(
                            span,
                            format!("expected a file path after `import`, found {}", other.describe()),
                        );
                        Err(Failed)
                    }
                }
            }
            Tok::At => {
                let span = self.span();
                self.error(span, "pragmas must come before everything else in the file");
                Err(Failed)
            }
            _ => self.stmt().map(Item::Stmt),
        }
    }

    fn fn_decl(&mut self) -> PResult<FnDecl> {
        let lo = self.bump().span; // fn
        let name = self.ident("a function name")?;
        self.expect(&Tok::LParen, "after the function name")?;
        let mut params = Vec::new();
        while !self.at(&Tok::RParen) {
            let pname = self.ident("a parameter name")?;
            let ty = if self.eat(&Tok::Colon) {
                Some(self.type_expr()?)
            } else {
                None
            };
            params.push(Param { name: pname, ty });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RParen, "after the parameters")?;
        let ret = if self.eat(&Tok::Arrow) {
            Some(self.type_expr()?)
        } else {
            None
        };
        let body = self.block()?;
        let span = lo.to(body.span);
        Ok(FnDecl {
            name,
            params,
            ret,
            body,
            span,
        })
    }

    fn type_decl(&mut self) -> PResult<TypeDecl> {
        let lo = self.bump().span; // type
        let name = self.ident("a type name")?;
        self.expect(&Tok::Assign, "after the type name")?;
        let ty = self.type_expr()?;
        let span = lo.to(self.prev_span());
        Ok(TypeDecl { name, ty, span })
    }

    fn enum_decl(&mut self) -> PResult<EnumDecl> {
        let lo = self.bump().span; // enum
        let name = self.ident("an enum name")?;
        self.expect(&Tok::LBrace, "after the enum name")?;
        let mut variants = Vec::new();
        self.skip_newlines();
        while !self.at(&Tok::RBrace) {
            variants.push(self.ident("a variant name")?);
            self.skip_newlines();
            if !self.eat(&Tok::Comma) {
                break;
            }
            self.skip_newlines();
        }
        self.skip_newlines();
        let hi = self.expect(&Tok::RBrace, "to close the enum")?;
        Ok(EnumDecl {
            name,
            variants,
            span: lo.to(hi),
        })
    }

    fn type_expr(&mut self) -> PResult<TypeExpr> {
        if self.at(&Tok::LBrace) {
            let lo = self.bump().span;
            let mut fields = Vec::new();
            self.skip_newlines();
            while !self.at(&Tok::RBrace) {
                let name = self.ident("a field name")?;
                self.expect(&Tok::Colon, "after the field name")?;
                let ty = self.type_expr()?;
                fields.push((name, ty));
                self.skip_newlines();
                if !self.eat(&Tok::Comma) {
                    break;
                }
                self.skip_newlines();
            }
            self.skip_newlines();
            let hi = self.expect(&Tok::RBrace, "to close the record type")?;
            return Ok(TypeExpr::Record {
                fields,
                span: lo.to(hi),
            });
        }
        let name = self.ident("a type")?;
        let mut args = Vec::new();
        if self.eat(&Tok::LBracket) {
            loop {
                args.push(self.type_expr()?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
            self.expect(&Tok::RBracket, "to close the type arguments")?;
        }
        Ok(TypeExpr::Named { name, args })
    }

    // ── Statements ───────────────────────────────────────────────────────

    fn block(&mut self) -> PResult<Block> {
        self.nested(|p| p.block_inner())
    }

    fn block_inner(&mut self) -> PResult<Block> {
        let lo = self.expect(&Tok::LBrace, "to start a block")?;
        self.with_restriction(false, |p| {
            let mut stmts = Vec::new();
            loop {
                p.skip_separators();
                match p.peek() {
                    Tok::RBrace => break,
                    Tok::Eof => {
                        p.error(lo, "this `{` is never closed");
                        return Err(Failed);
                    }
                    _ => {}
                }
                match p.stmt() {
                    Ok(stmt) => {
                        stmts.push(stmt);
                        if p.end_of_statement().is_err() {
                            p.recover();
                        }
                    }
                    Err(Failed) => p.recover(),
                }
            }
            let hi = p.bump().span; // }
            Ok(Block { stmts, span: lo.to(hi) })
        })
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let lo = self.span();
        let kind = match self.peek() {
            Tok::Let | Tok::Var => {
                let mutable = self.bump().tok == Tok::Var;
                let pattern = self.pattern()?;
                let ty = if self.eat(&Tok::Colon) {
                    Some(self.type_expr()?)
                } else {
                    None
                };
                let op = match self.peek() {
                    Tok::Assign => BindOp::Assign,
                    Tok::Tilde => BindOp::Draw,
                    other => {
                        let found = other.describe();
                        let span = self.span();
                        self.error(span, format!("expected `=` or `~` in the binding, found {found}"))
                            .help("`=` keeps a value; `~` draws one from a distribution");
                        return Err(Failed);
                    }
                };
                self.bump();
                let value = self.expr()?;
                StmtKind::Let {
                    mutable,
                    pattern,
                    ty,
                    op,
                    value,
                }
            }
            Tok::For => {
                self.bump();
                let pattern = self.pattern()?;
                self.expect(&Tok::In, "after the loop variable")?;
                let iter = self.with_restriction(true, |p| p.expr())?;
                let body = self.block()?;
                StmtKind::For { pattern, iter, body }
            }
            Tok::While => {
                self.bump();
                let cond = self.with_restriction(true, |p| p.expr())?;
                let body = self.block()?;
                StmtKind::While { cond, body }
            }
            Tok::Repeat => {
                self.bump();
                let count = self.with_restriction(true, |p| p.expr())?;
                let body = self.block()?;
                StmtKind::Repeat { count, body }
            }
            Tok::Loop => {
                self.bump();
                let body = self.block()?;
                StmtKind::Loop { body }
            }
            Tok::Break => {
                self.bump();
                StmtKind::Break
            }
            Tok::Continue => {
                self.bump();
                StmtKind::Continue
            }
            Tok::Return => {
                self.bump();
                if matches!(
                    self.peek(),
                    Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof | Tok::Comma
                ) {
                    StmtKind::Return(None)
                } else {
                    StmtKind::Return(Some(self.expr()?))
                }
            }
            Tok::Observe => {
                self.bump();
                let value = self.expr()?;
                let from = if self.peek().is_ident("from") {
                    self.bump();
                    Some(self.expr()?)
                } else {
                    None
                };
                StmtKind::Observe { value, from }
            }
            Tok::Report => {
                self.bump();
                let value = self.expr()?;
                let by = if self.peek().is_ident("by") {
                    self.bump();
                    Some(self.expr()?)
                } else {
                    None
                };
                let label = if self.peek().is_ident("as") {
                    self.bump();
                    match self.peek().clone() {
                        Tok::Str(parts) => {
                            let span = self.bump().span;
                            match plain_string(&parts) {
                                Some(text) => Some((text, span)),
                                None => {
                                    self.error(span, "a report label can't contain `{…}`");
                                    return Err(Failed);
                                }
                            }
                        }
                        other => {
                            let span = self.span();
                            self.error(
                                span,
                                format!("expected a label string after `as`, found {}", other.describe()),
                            );
                            return Err(Failed);
                        }
                    }
                } else {
                    None
                };
                StmtKind::Report { value, by, label }
            }
            Tok::Fn | Tok::Type | Tok::Enum | Tok::Import => {
                let span = self.span();
                let what = self.peek().text();
                self.error(span, format!("`{what}` declarations are only allowed at the top level"));
                return Err(Failed);
            }
            _ => {
                let target = self.expr()?;
                let op = match self.peek() {
                    Tok::Assign => Some(AssignOp::Set),
                    Tok::Tilde => Some(AssignOp::Draw),
                    Tok::PlusAssign => Some(AssignOp::Add),
                    Tok::MinusAssign => Some(AssignOp::Sub),
                    Tok::StarAssign => Some(AssignOp::Mul),
                    Tok::SlashAssign => Some(AssignOp::Div),
                    _ => None,
                };
                match op {
                    None => StmtKind::Expr(target),
                    Some(op) => {
                        self.bump();
                        if !is_place(&target) {
                            self.error(target.span, "can't assign to this")
                                .help("only variables, fields (`a.b`) and elements (`a[i]`) can be assigned");
                            return Err(Failed);
                        }
                        let value = self.expr()?;
                        StmtKind::Assign { target, op, value }
                    }
                }
            }
        };
        Ok(Stmt {
            kind,
            span: lo.to(self.prev_span()),
        })
    }

    // ── Patterns ─────────────────────────────────────────────────────────

    fn pattern(&mut self) -> PResult<Pattern> {
        self.nested(|p| p.pattern_inner())
    }

    fn pattern_inner(&mut self) -> PResult<Pattern> {
        let first = self.pattern_alt()?;
        if !self.at(&Tok::Pipe) {
            return Ok(first);
        }
        let mut alts = vec![first];
        while self.eat(&Tok::Pipe) {
            alts.push(self.pattern_alt()?);
        }
        let span = alts[0].span.to(alts.last().unwrap().span);
        Ok(Pattern {
            kind: PatternKind::Or(alts),
            span,
        })
    }

    fn pattern_alt(&mut self) -> PResult<Pattern> {
        let span = self.span();
        let kind = match self.peek().clone() {
            Tok::Underscore => {
                self.bump();
                PatternKind::Wildcard
            }
            Tok::Ident(name) => {
                self.bump();
                PatternKind::Name(name)
            }
            Tok::Int(_) | Tok::Float(_) | Tok::Percent(_) | Tok::Str(_) | Tok::True | Tok::False => {
                PatternKind::Literal(self.primary()?)
            }
            Tok::Minus if matches!(self.peek_at(1), Tok::Int(_) | Tok::Float(_) | Tok::Percent(_)) => {
                self.bump();
                let inner = self.primary()?;
                PatternKind::Literal(Expr {
                    span: span.to(inner.span),
                    kind: ExprKind::Unary {
                        op: UnOp::Neg,
                        expr: Box::new(inner),
                    },
                })
            }
            Tok::LBracket => {
                self.bump();
                let mut items = Vec::new();
                while !self.at(&Tok::RBracket) {
                    items.push(self.pattern()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(&Tok::RBracket, "to close the list pattern")?;
                PatternKind::List(items)
            }
            other => {
                self.error(span, format!("expected a pattern, found {}", other.describe()));
                return Err(Failed);
            }
        };
        Ok(Pattern {
            kind,
            span: span.to(self.prev_span()),
        })
    }

    // ── Expressions ──────────────────────────────────────────────────────

    fn expr(&mut self) -> PResult<Expr> {
        if let Some(lambda) = self.lambda()? {
            return Ok(lambda);
        }
        self.expr_bp(0)
    }

    /// `x -> body`, `(a, b) -> body` or `() -> body`, if one starts here.
    fn lambda(&mut self) -> PResult<Option<Expr>> {
        let lo = self.span();
        let params = match (self.peek(), self.peek_at(1)) {
            (Tok::Ident(_), Tok::Arrow) => {
                let param = self.ident("a parameter")?;
                vec![param]
            }
            (Tok::LParen, _) => {
                // Look for `( ident, … ) ->` without consuming anything.
                let mut i = 1;
                let mut expect_ident = true;
                loop {
                    match (self.peek_at(i), expect_ident) {
                        (Tok::RParen, _) => break,
                        (Tok::Ident(_), true) => expect_ident = false,
                        (Tok::Comma, false) => expect_ident = true,
                        _ => return Ok(None),
                    }
                    i += 1;
                }
                if *self.peek_at(i + 1) != Tok::Arrow {
                    return Ok(None);
                }
                self.bump(); // (
                let mut params = Vec::new();
                while !self.at(&Tok::RParen) {
                    params.push(self.ident("a parameter")?);
                    self.eat(&Tok::Comma);
                }
                self.bump(); // )
                params
            }
            _ => return Ok(None),
        };
        self.expect(&Tok::Arrow, "")?;
        let body = self.expr()?;
        let span = lo.to(body.span);
        Ok(Some(Expr {
            kind: ExprKind::Lambda {
                params,
                body: Box::new(body),
            },
            span,
        }))
    }

    fn expr_bp(&mut self, min: u8) -> PResult<Expr> {
        self.nested(|p| p.expr_bp_inner(min))
    }

    fn expr_bp_inner(&mut self, min: u8) -> PResult<Expr> {
        let lo = self.span();
        let mut lhs = match self.peek() {
            Tok::Typeof => {
                self.bump();
                let operand = self.expr_bp(PREC_TYPEOF)?;
                Expr {
                    span: lo.to(operand.span),
                    kind: ExprKind::Unary {
                        op: UnOp::Typeof,
                        expr: Box::new(operand),
                    },
                }
            }
            Tok::Not => {
                self.bump();
                let operand = self.expr_bp(PREC_NOT)?;
                Expr {
                    span: lo.to(operand.span),
                    kind: ExprKind::Unary {
                        op: UnOp::Not,
                        expr: Box::new(operand),
                    },
                }
            }
            Tok::Minus => {
                self.bump();
                let operand = self.expr_bp(PREC_NEG)?;
                Expr {
                    span: lo.to(operand.span),
                    kind: ExprKind::Unary {
                        op: UnOp::Neg,
                        expr: Box::new(operand),
                    },
                }
            }
            _ => self.postfix()?,
        };
        while let Some((op, prec, len)) = self.binary_op() {
            if prec < min {
                break;
            }
            for _ in 0..len {
                self.bump();
            }
            let rhs = match prec {
                PREC_POW => self.expr_bp(PREC_POW)?, // right-associative
                _ => self.expr_bp(prec + 1)?,
            };
            if prec == PREC_CMP || prec == PREC_RANGE {
                if let Some((next, next_prec, _)) = self.binary_op() {
                    if next_prec == prec {
                        let span = self.span();
                        let msg = if prec == PREC_CMP {
                            format!(
                                "comparisons can't be chained: `{}` after `{}`",
                                next.symbol(),
                                op.symbol()
                            )
                        } else {
                            format!("`{}` can't follow `{}` directly", next.symbol(), op.symbol())
                        };
                        self.error(span, msg)
                            .help("add parentheses, or combine the tests with `and`");
                        return Err(Failed);
                    }
                }
            }
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    /// The binary operator at the current position: (operator, precedence, token count).
    fn binary_op(&self) -> Option<(BinOp, u8, usize)> {
        Some(match self.peek() {
            Tok::Or => (BinOp::Or, PREC_OR, 1),
            Tok::And => (BinOp::And, PREC_AND, 1),
            Tok::EqEq => (BinOp::Eq, PREC_CMP, 1),
            Tok::NotEq => (BinOp::Ne, PREC_CMP, 1),
            Tok::Lt => (BinOp::Lt, PREC_CMP, 1),
            Tok::Le => (BinOp::Le, PREC_CMP, 1),
            Tok::Gt => (BinOp::Gt, PREC_CMP, 1),
            Tok::Ge => (BinOp::Ge, PREC_CMP, 1),
            Tok::In => (BinOp::In, PREC_CMP, 1),
            Tok::Not if *self.peek_at(1) == Tok::In => (BinOp::NotIn, PREC_CMP, 2),
            Tok::DotDot => (BinOp::Range, PREC_RANGE, 1),
            Tok::DotDotLt => (BinOp::RangeExcl, PREC_RANGE, 1),
            Tok::Ident(word) if word == "to" => (BinOp::To, PREC_RANGE, 1),
            Tok::Plus => (BinOp::Add, PREC_ADD, 1),
            Tok::Minus => (BinOp::Sub, PREC_ADD, 1),
            Tok::Star => (BinOp::Mul, PREC_MUL, 1),
            Tok::Slash => (BinOp::Div, PREC_MUL, 1),
            Tok::Div => (BinOp::IntDiv, PREC_MUL, 1),
            Tok::Mod => (BinOp::Mod, PREC_MUL, 1),
            Tok::Caret => (BinOp::Pow, PREC_POW, 1),
            _ => return None,
        })
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut expr = self.primary()?;
        loop {
            match self.peek() {
                Tok::Dot => {
                    self.bump();
                    let name = self.ident("a field or method name after `.`")?;
                    if self.at(&Tok::LParen) {
                        let args = self.call_args()?;
                        let span = expr.span.to(self.prev_span());
                        expr = Expr {
                            kind: ExprKind::Method {
                                receiver: Box::new(expr),
                                name,
                                args,
                            },
                            span,
                        };
                    } else {
                        let span = expr.span.to(name.span);
                        expr = Expr {
                            kind: ExprKind::Field {
                                expr: Box::new(expr),
                                name,
                            },
                            span,
                        };
                    }
                }
                Tok::LParen => {
                    let args = self.call_args()?;
                    let span = expr.span.to(self.prev_span());
                    expr = Expr {
                        kind: ExprKind::Call {
                            callee: Box::new(expr),
                            args,
                        },
                        span,
                    };
                }
                Tok::LBracket => {
                    self.bump();
                    let index = self.with_restriction(false, |p| p.expr())?;
                    let hi = self.expect(&Tok::RBracket, "to close the index")?;
                    let span = expr.span.to(hi);
                    expr = Expr {
                        kind: ExprKind::Index {
                            expr: Box::new(expr),
                            index: Box::new(index),
                        },
                        span,
                    };
                }
                Tok::With => {
                    self.bump();
                    self.expect(&Tok::LBrace, "after `with`")?;
                    let fields = self.record_fields()?;
                    let span = expr.span.to(self.prev_span());
                    expr = Expr {
                        kind: ExprKind::With {
                            expr: Box::new(expr),
                            fields,
                        },
                        span,
                    };
                }
                _ => return Ok(expr),
            }
        }
    }

    fn call_args(&mut self) -> PResult<Vec<Arg>> {
        self.expect(&Tok::LParen, "")?;
        self.with_restriction(false, |p| {
            let mut args = Vec::new();
            while !p.at(&Tok::RParen) {
                let name = if matches!(p.peek(), Tok::Ident(_)) && *p.peek_at(1) == Tok::Colon {
                    let name = p.ident("an argument name")?;
                    p.bump(); // :
                    Some(name)
                } else {
                    None
                };
                let value = p.expr()?;
                args.push(Arg { name, value });
                if !p.eat(&Tok::Comma) {
                    break;
                }
            }
            p.expect(&Tok::RParen, "to close the arguments")?;
            Ok(args)
        })
    }

    /// Fields of a record literal, after its `{`; consumes the closing `}`.
    fn record_fields(&mut self) -> PResult<Vec<Field>> {
        self.with_restriction(false, |p| {
            let mut fields = Vec::new();
            p.skip_newlines();
            while !p.at(&Tok::RBrace) {
                let name = p.ident("a field name")?;
                let value = if p.eat(&Tok::Colon) {
                    p.expr()?
                } else {
                    Expr {
                        kind: ExprKind::Name(name.name.clone()),
                        span: name.span,
                    }
                };
                fields.push(Field { name, value });
                p.skip_newlines();
                if !p.eat(&Tok::Comma) {
                    break;
                }
                p.skip_newlines();
            }
            p.skip_newlines();
            p.expect(&Tok::RBrace, "to close the record")?;
            Ok(fields)
        })
    }

    /// Does the `{` at `self.pos + offset` start a record rather than a block?
    fn record_ahead(&self, offset: usize, allow_empty: bool) -> bool {
        let mut i = offset + 1;
        while *self.peek_at(i) == Tok::Newline {
            i += 1;
        }
        match (self.peek_at(i), self.peek_at(i + 1)) {
            (Tok::RBrace, _) => allow_empty,
            (Tok::Ident(_), Tok::Colon | Tok::Comma) => true,
            (Tok::Ident(_), Tok::RBrace) => allow_empty,
            _ => false,
        }
    }

    fn primary(&mut self) -> PResult<Expr> {
        let lo = self.span();
        let kind = match self.peek().clone() {
            Tok::Int(v) => {
                self.bump();
                ExprKind::Int(v)
            }
            Tok::Float(v) => {
                self.bump();
                ExprKind::Float(v)
            }
            Tok::Percent(v) => {
                self.bump();
                ExprKind::Percent(v)
            }
            Tok::Dice { count, sides } => {
                self.bump();
                ExprKind::Dice { count, sides }
            }
            Tok::True => {
                self.bump();
                ExprKind::Bool(true)
            }
            Tok::False => {
                self.bump();
                ExprKind::Bool(false)
            }
            Tok::Str(parts) => {
                self.bump();
                ExprKind::Str(self.string_segments(parts))
            }
            Tok::Ident(name) => {
                // A typed record literal: `Fighter { hp: 12 }`.
                if !self.restricted && *self.peek_at(1) == Tok::LBrace && self.record_ahead(1, true) {
                    let ident = self.ident("")?;
                    self.bump(); // {
                    let fields = self.record_fields()?;
                    ExprKind::Record {
                        name: Some(ident),
                        fields,
                    }
                } else {
                    self.bump();
                    ExprKind::Name(name)
                }
            }
            Tok::LParen => {
                self.bump();
                let inner = self.with_restriction(false, |p| p.expr())?;
                self.expect(&Tok::RParen, "to close the parenthesis")?;
                let span = lo.to(self.prev_span());
                return Ok(Expr { span, ..inner });
            }
            Tok::LBracket => return self.list_or_map(),
            Tok::LBrace => {
                if self.restricted {
                    self.error(lo, "expected an expression before `{`");
                    return Err(Failed);
                }
                if self.record_ahead(0, false) {
                    self.bump();
                    let fields = self.record_fields()?;
                    ExprKind::Record { name: None, fields }
                } else {
                    ExprKind::Block(self.block()?)
                }
            }
            Tok::If => return self.if_expr(),
            Tok::Chance => return self.chance_expr(),
            Tok::Match => return self.match_expr(),
            Tok::Simulate => {
                self.bump();
                ExprKind::Simulate(self.block()?)
            }
            Tok::Underscore => {
                self.error(lo, "`_` can only be used in patterns");
                return Err(Failed);
            }
            other => {
                self.error(lo, format!("expected an expression, found {}", other.describe()));
                return Err(Failed);
            }
        };
        Ok(Expr {
            kind,
            span: lo.to(self.prev_span()),
        })
    }

    fn string_segments(&mut self, parts: Vec<StrPart>) -> Vec<StrSegment> {
        let mut segments = Vec::new();
        for part in parts {
            match part {
                StrPart::Lit(text) => segments.push(StrSegment::Lit(text)),
                StrPart::Expr { src, offset } => {
                    let (expr, mut diags) = parse_nested_expr(&src, offset, self.depth + 1);
                    self.diags.append(&mut diags);
                    if let Some(expr) = expr {
                        segments.push(StrSegment::Expr(expr));
                    }
                }
            }
        }
        segments
    }

    fn list_or_map(&mut self) -> PResult<Expr> {
        let lo = self.bump().span; // [
        self.with_restriction(false, |p| {
            if p.at(&Tok::Colon) && *p.peek_at(1) == Tok::RBracket {
                p.bump();
                let hi = p.bump().span;
                return Ok(Expr {
                    kind: ExprKind::Map(Vec::new()),
                    span: lo.to(hi),
                });
            }
            if p.at(&Tok::RBracket) {
                let hi = p.bump().span;
                return Ok(Expr {
                    kind: ExprKind::List(Vec::new()),
                    span: lo.to(hi),
                });
            }
            let first = p.expr()?;
            if p.eat(&Tok::Colon) {
                let value = p.expr()?;
                let mut entries = vec![(first, value)];
                while p.eat(&Tok::Comma) {
                    if p.at(&Tok::RBracket) {
                        break;
                    }
                    let key = p.expr()?;
                    p.expect(&Tok::Colon, "between a key and its value")?;
                    let value = p.expr()?;
                    entries.push((key, value));
                }
                let hi = p.expect(&Tok::RBracket, "to close the map")?;
                Ok(Expr {
                    kind: ExprKind::Map(entries),
                    span: lo.to(hi),
                })
            } else {
                let mut items = vec![first];
                while p.eat(&Tok::Comma) {
                    if p.at(&Tok::RBracket) {
                        break;
                    }
                    items.push(p.expr()?);
                }
                let hi = p.expect(&Tok::RBracket, "to close the list")?;
                Ok(Expr {
                    kind: ExprKind::List(items),
                    span: lo.to(hi),
                })
            }
        })
    }

    fn if_expr(&mut self) -> PResult<Expr> {
        let lo = self.bump().span; // if
        let cond = self.with_restriction(true, |p| p.expr())?;
        let then = self.block()?;
        // Allow `else` on the line after the closing `}` (but not `else =>`,
        // which starts the next arm of a `chance` block).
        let mut ahead = 0;
        while *self.peek_at(ahead) == Tok::Newline {
            ahead += 1;
        }
        if ahead > 0 && *self.peek_at(ahead) == Tok::Else && matches!(self.peek_at(ahead + 1), Tok::LBrace | Tok::If) {
            self.skip_newlines();
        }
        let otherwise = if self.eat(&Tok::Else) {
            if self.at(&Tok::If) {
                Some(Box::new(self.nested(|p| p.if_expr())?))
            } else {
                let block = self.block()?;
                Some(Box::new(Expr {
                    span: block.span,
                    kind: ExprKind::Block(block),
                }))
            }
        } else {
            None
        };
        Ok(Expr {
            kind: ExprKind::If {
                cond: Box::new(cond),
                then,
                otherwise,
            },
            span: lo.to(self.prev_span()),
        })
    }

    /// Arms of a `chance` or `match` body: `{ arm, arm \n arm }`.
    fn arms<T>(&mut self, mut arm: impl FnMut(&mut Parser) -> PResult<T>) -> PResult<Vec<T>> {
        self.expect(&Tok::LBrace, "to start the arms")?;
        self.with_restriction(false, |p| {
            let mut arms = Vec::new();
            loop {
                while matches!(p.peek(), Tok::Newline | Tok::Comma) {
                    p.bump();
                }
                if p.at(&Tok::RBrace) {
                    break;
                }
                match arm(p) {
                    Ok(a) => arms.push(a),
                    Err(Failed) => {
                        // Skip to the next arm.
                        while !matches!(p.peek(), Tok::Newline | Tok::Comma | Tok::RBrace | Tok::Eof) {
                            p.bump();
                        }
                        if p.at(&Tok::Eof) {
                            return Err(Failed);
                        }
                        continue;
                    }
                }
                match p.peek() {
                    Tok::Newline | Tok::Comma | Tok::RBrace => {}
                    other => {
                        let found = other.describe();
                        let span = p.span();
                        p.error(span, format!("expected `,` or a new line between arms, found {found}"));
                        return Err(Failed);
                    }
                }
            }
            p.bump(); // }
            Ok(arms)
        })
    }

    fn chance_expr(&mut self) -> PResult<Expr> {
        let lo = self.bump().span; // chance
        let arms = self.arms(|p| {
            let arm_lo = p.span();
            let weight = if p.eat(&Tok::Else) { None } else { Some(p.expr()?) };
            p.expect(&Tok::FatArrow, "after the arm's probability")?;
            let body = p.stmt()?;
            Ok(ChanceArm {
                weight,
                span: arm_lo.to(body.span),
                body,
            })
        })?;
        Ok(Expr {
            kind: ExprKind::Chance { arms },
            span: lo.to(self.prev_span()),
        })
    }

    fn match_expr(&mut self) -> PResult<Expr> {
        let lo = self.bump().span; // match
        let scrutinee = self.with_restriction(true, |p| p.expr())?;
        let arms = self.arms(|p| {
            let pattern = p.pattern()?;
            let guard = if p.eat(&Tok::If) { Some(p.expr()?) } else { None };
            p.expect(&Tok::FatArrow, "after the pattern")?;
            let body = p.stmt()?;
            Ok(MatchArm {
                span: pattern.span.to(body.span),
                pattern,
                guard,
                body,
            })
        })?;
        Ok(Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            span: lo.to(self.prev_span()),
        })
    }
}

/// The text of a string literal without interpolation.
fn plain_string(parts: &[StrPart]) -> Option<String> {
    let mut text = String::new();
    for part in parts {
        match part {
            StrPart::Lit(s) => text.push_str(s),
            StrPart::Expr { .. } => return None,
        }
    }
    Some(text)
}

fn is_place(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Name(_) => true,
        ExprKind::Field { expr, .. } | ExprKind::Index { expr, .. } => is_place(expr),
        _ => false,
    }
}
