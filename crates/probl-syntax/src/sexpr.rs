//! A compact S-expression rendering of the AST, used in tests.

use crate::ast::*;
use std::fmt::Write;

pub fn program(p: &Program) -> String {
    let mut out = String::new();
    for pragma in &p.pragmas {
        match &pragma.arg {
            Some(arg) => writeln!(out, "(@{} {})", pragma.name.name, expr(arg)).unwrap(),
            None => writeln!(out, "(@{})", pragma.name.name).unwrap(),
        }
    }
    for item in &p.items {
        writeln!(out, "{}", self::item(item)).unwrap();
    }
    out
}

pub fn item(item: &Item) -> String {
    match item {
        Item::Fn(f) => {
            let params: Vec<String> = f
                .params
                .iter()
                .map(|p| match &p.ty {
                    Some(t) => format!("{}: {}", p.name.name, ty(t)),
                    None => p.name.name.clone(),
                })
                .collect();
            let ret = f.ret.as_ref().map(|t| format!(" -> {}", ty(t))).unwrap_or_default();
            format!("(fn {} ({}){} {})", f.name.name, params.join(", "), ret, block(&f.body))
        }
        Item::Type(t) => format!("(type {} {})", t.name.name, ty(&t.ty)),
        Item::Enum(e) => {
            let names: Vec<&str> = e.variants.iter().map(|v| v.name.as_str()).collect();
            format!("(enum {} {})", e.name.name, names.join(" "))
        }
        Item::Import(i) => format!("(import {:?})", i.path),
        Item::Stmt(s) => stmt(s),
    }
}

pub fn ty(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Named { name, args } if args.is_empty() => name.name.clone(),
        TypeExpr::Named { name, args } => {
            let args: Vec<String> = args.iter().map(ty).collect();
            format!("{}[{}]", name.name, args.join(", "))
        }
        TypeExpr::Record { fields, .. } => {
            let fields: Vec<String> = fields.iter().map(|(n, t)| format!("{}: {}", n.name, ty(t))).collect();
            format!("{{{}}}", fields.join(", "))
        }
    }
}

pub fn block(b: &Block) -> String {
    let stmts: Vec<String> = b.stmts.iter().map(stmt).collect();
    if stmts.is_empty() {
        "(block)".to_string()
    } else {
        format!("(block {})", stmts.join(" "))
    }
}

pub fn stmt(s: &Stmt) -> String {
    match &s.kind {
        StmtKind::Let {
            mutable,
            pattern: p,
            ty: t,
            op,
            value,
        } => {
            let kw = if *mutable { "var" } else { "let" };
            let op = match op {
                BindOp::Assign => "=",
                BindOp::Draw => "~",
            };
            let t = t.as_ref().map(|t| format!(": {}", ty(t))).unwrap_or_default();
            format!("({kw} {}{t} {op} {})", pattern(p), expr(value))
        }
        StmtKind::Assign { target, op, value } => {
            let op = match op {
                AssignOp::Set => "=",
                AssignOp::Draw => "~",
                AssignOp::Add => "+=",
                AssignOp::Sub => "-=",
                AssignOp::Mul => "*=",
                AssignOp::Div => "/=",
            };
            format!("({op} {} {})", expr(target), expr(value))
        }
        StmtKind::For { pattern: p, iter, body } => format!("(for {} {} {})", pattern(p), expr(iter), block(body)),
        StmtKind::While { cond, body } => format!("(while {} {})", expr(cond), block(body)),
        StmtKind::Repeat { count, body } => format!("(repeat {} {})", expr(count), block(body)),
        StmtKind::Loop { body } => format!("(loop {})", block(body)),
        StmtKind::Break => "(break)".to_string(),
        StmtKind::Continue => "(continue)".to_string(),
        StmtKind::Return(None) => "(return)".to_string(),
        StmtKind::Return(Some(e)) => format!("(return {})", expr(e)),
        StmtKind::Observe { value, from } => match from {
            Some(d) => format!("(observe {} from {})", expr(value), expr(d)),
            None => format!("(observe {})", expr(value)),
        },
        StmtKind::Report { value, by, label } => {
            let mut out = format!("(report {}", expr(value));
            if let Some(by) = by {
                write!(out, " by {}", expr(by)).unwrap();
            }
            if let Some((label, _)) = label {
                write!(out, " as {label:?}").unwrap();
            }
            out.push(')');
            out
        }
        StmtKind::Expr(e) => expr(e),
    }
}

pub fn pattern(p: &Pattern) -> String {
    match &p.kind {
        PatternKind::Wildcard => "_".to_string(),
        PatternKind::Name(n) => n.clone(),
        PatternKind::Literal(e) => expr(e),
        PatternKind::List(items) => {
            let items: Vec<String> = items.iter().map(pattern).collect();
            format!("[{}]", items.join(" "))
        }
        PatternKind::Or(alts) => {
            let alts: Vec<String> = alts.iter().map(pattern).collect();
            format!("(| {})", alts.join(" "))
        }
    }
}

fn args(args: &[Arg]) -> String {
    args.iter()
        .map(|a| match &a.name {
            Some(n) => format!(" {}: {}", n.name, expr(&a.value)),
            None => format!(" {}", expr(&a.value)),
        })
        .collect()
}

fn fields(fields: &[Field]) -> String {
    fields
        .iter()
        .map(|f| format!(" ({} {})", f.name.name, expr(&f.value)))
        .collect()
}

pub fn percent(v: f64) -> String {
    let pct = (v * 100.0 * 1e9).round() / 1e9;
    format!("{pct}%")
}

pub fn expr(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int(v) => v.to_string(),
        ExprKind::Float(v) => format!("{v:?}"),
        ExprKind::Percent(v) => percent(*v),
        ExprKind::Dice { count, sides } => format!("{count}d{sides}"),
        ExprKind::Bool(b) => b.to_string(),
        ExprKind::Str(segments) => {
            if let [StrSegment::Lit(text)] = segments.as_slice() {
                return format!("{text:?}");
            }
            let parts: Vec<String> = segments
                .iter()
                .map(|s| match s {
                    StrSegment::Lit(text) => format!("{text:?}"),
                    StrSegment::Expr(e) => expr(e),
                })
                .collect();
            format!("(str {})", parts.join(" "))
        }
        ExprKind::Name(n) => n.clone(),
        ExprKind::List(items) => {
            let items: Vec<String> = items.iter().map(expr).collect();
            format!("[{}]", items.join(" "))
        }
        ExprKind::Map(entries) => {
            let entries: Vec<String> = entries
                .iter()
                .map(|(k, v)| format!("{}: {}", expr(k), expr(v)))
                .collect();
            format!(
                "[{}]",
                if entries.is_empty() {
                    ":".to_string()
                } else {
                    entries.join(", ")
                }
            )
        }
        ExprKind::Record { name, fields: fs } => match name {
            Some(n) => format!("(record {}{})", n.name, fields(fs)),
            None => format!("(record{})", fields(fs)),
        },
        ExprKind::Unary { op, expr: inner } => {
            let op = match op {
                UnOp::Neg => "-",
                UnOp::Not => "not",
                UnOp::Typeof => "typeof",
            };
            format!("({op} {})", expr(inner))
        }
        ExprKind::Binary { op, lhs, rhs } => {
            format!("({} {} {})", op.symbol(), expr(lhs), expr(rhs))
        }
        ExprKind::Call { callee, args: a } => format!("(call {}{})", expr(callee), args(a)),
        ExprKind::Method {
            receiver,
            name,
            args: a,
        } => format!("(.{} {}{})", name.name, expr(receiver), args(a)),
        ExprKind::Field { expr: inner, name } => format!("(. {} {})", expr(inner), name.name),
        ExprKind::Index { expr: inner, index } => {
            format!("(index {} {})", expr(inner), expr(index))
        }
        ExprKind::With {
            expr: inner,
            fields: fs,
        } => format!("(with {}{})", expr(inner), fields(fs)),
        ExprKind::Lambda { params, body } => {
            let params: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
            format!("(-> ({}) {})", params.join(" "), expr(body))
        }
        ExprKind::If { cond, then, otherwise } => match otherwise {
            Some(other) => format!("(if {} {} {})", expr(cond), block(then), expr(other)),
            None => format!("(if {} {})", expr(cond), block(then)),
        },
        ExprKind::Chance { arms } => {
            let arms: Vec<String> = arms
                .iter()
                .map(|a| {
                    let w = a.weight.as_ref().map(expr).unwrap_or_else(|| "else".to_string());
                    format!("({w} => {})", stmt(&a.body))
                })
                .collect();
            format!("(chance {})", arms.join(" "))
        }
        ExprKind::Match { scrutinee, arms } => {
            let arms: Vec<String> = arms
                .iter()
                .map(|a| {
                    let guard = a.guard.as_ref().map(|g| format!(" if {}", expr(g))).unwrap_or_default();
                    format!("({}{guard} => {})", pattern(&a.pattern), stmt(&a.body))
                })
                .collect();
            format!("(match {} {})", expr(scrutinee), arms.join(" "))
        }
        ExprKind::Simulate(b) => format!("(simulate {})", block(b)),
        ExprKind::Block(b) => block(b),
    }
}
