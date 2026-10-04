//! The middle of the Probl pipeline: name resolution, lowering to IR and
//! liveness analysis.

pub mod builtins;
pub mod coercions;
pub mod conjugate;
pub mod data;
pub mod docs;
mod draws;
pub mod effects;
pub mod ir;
pub mod liveness;
pub mod lower;
pub mod pretty;
pub mod symbols;

pub use builtins::{Builtin, Constant};
pub use liveness::{Liveness, SlotSet, analyze};
pub use lower::{lower, lower_with_symbols};

use probl_syntax::{Diagnostic, parse_program};

/// The largest source text `compile` accepts, in bytes.
pub const MAX_SOURCE: usize = 8 * 1024 * 1024;

/// Parse and lower a program, and say where its names are declared and
/// used. The symbols are there if it parses, even with other errors.
pub fn compile_with_symbols(src: &str) -> (Option<ir::Program>, Vec<Diagnostic>, Option<symbols::Symbols>) {
    if src.len() > MAX_SOURCE {
        let d = Diagnostic::error(
            probl_syntax::Span::default(),
            format!("the program is larger than {MAX_SOURCE} bytes"),
        );
        return (None, vec![d], None);
    }
    let (ast, mut diags) = parse_program(src);
    if diags.iter().any(Diagnostic::is_error) {
        return (None, diags, None);
    }
    let (program, mut lower_diags, symbols) = lower_with_symbols(&ast, src);
    diags.append(&mut lower_diags);
    diags.sort_by_key(|d| d.span.lo);
    let program = (!diags.iter().any(Diagnostic::is_error)).then_some(program);
    (program, diags, Some(symbols))
}

/// Parse and lower a program. The IR is only returned when there are no
/// errors; warnings are returned either way.
pub fn compile(src: &str) -> (Option<ir::Program>, Vec<Diagnostic>) {
    let (program, diags, _) = compile_with_symbols(src);
    (program, diags)
}
