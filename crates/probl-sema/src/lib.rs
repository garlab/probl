//! The middle of the Probl pipeline: name resolution, lowering to IR and
//! liveness analysis.

pub mod builtins;
pub mod data;
mod draws;
pub mod effects;
pub mod ir;
pub mod liveness;
pub mod lower;
pub mod pretty;

pub use builtins::Builtin;
pub use liveness::{Liveness, SlotSet, analyze};
pub use lower::lower;

use probl_syntax::{Diagnostic, parse_program};

/// The largest source text `compile` accepts, in bytes.
pub const MAX_SOURCE: usize = 8 * 1024 * 1024;

/// Parse and lower a program. The IR is only returned when there are no
/// errors; warnings are returned either way.
pub fn compile(src: &str) -> (Option<ir::Program>, Vec<Diagnostic>) {
    if src.len() > MAX_SOURCE {
        let d = Diagnostic::error(
            probl_syntax::Span::default(),
            format!("the program is larger than {MAX_SOURCE} bytes"),
        );
        return (None, vec![d]);
    }
    let (ast, mut diags) = parse_program(src);
    if diags.iter().any(Diagnostic::is_error) {
        return (None, diags);
    }
    let (program, mut lower_diags) = lower(&ast, src);
    diags.append(&mut lower_diags);
    diags.sort_by_key(|d| d.span.lo);
    if diags.iter().any(Diagnostic::is_error) {
        (None, diags)
    } else {
        (Some(program), diags)
    }
}
