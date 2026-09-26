//! The middle of the Probl pipeline: name resolution, lowering to IR and
//! liveness analysis.

pub mod builtins;
pub mod ir;
pub mod liveness;
pub mod lower;
pub mod pretty;

pub use builtins::Builtin;
pub use liveness::{Liveness, SlotSet, analyze};
pub use lower::lower;

use probl_syntax::{Diagnostic, parse_program};

/// Parse and lower a program. The IR is only returned when there are no
/// errors; warnings are returned either way.
pub fn compile(src: &str) -> (Option<ir::Program>, Vec<Diagnostic>) {
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
