//! Recognize numeric literals for early contextual conversion and diagnostics.
use crate::ir::{Expr, ExprKind, Lit};
use probl_syntax::ast::UnOp;

pub fn numeric_literal(e: &Expr) -> Option<f64> {
    match &e.kind {
        ExprKind::Lit(Lit::Float(x)) => Some(*x),
        ExprKind::Lit(Lit::Int(n)) => Some(n.to_f64().unwrap_or(f64::INFINITY)),
        ExprKind::Unary(UnOp::Neg, x) => numeric_literal(x).map(|n| -n),
        _ => None,
    }
}

/// Keep exact integer literals out of float-to-int validation: even converting
/// them to f64 just for inspection can round or overflow a valid bigint.
pub fn float_literal(e: &Expr) -> Option<f64> {
    match &e.kind {
        ExprKind::Lit(Lit::Float(x)) => Some(*x),
        ExprKind::Unary(UnOp::Neg, x) => float_literal(x).map(|n| -n),
        _ => None,
    }
}
