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
pub mod modules;
pub mod pretty;
pub mod rewards;
pub mod symbols;
pub mod types;

pub use builtins::{Builtin, Constant};
pub use liveness::{Liveness, SlotSet, analyze};
pub use lower::{lower, lower_with_symbols};
pub use modules::{Compiled, Loader, NoImports, Source, compile_program};

use probl_syntax::Diagnostic;

/// The largest source text `compile` accepts, in bytes.
pub const MAX_SOURCE: usize = modules::MAX_SOURCE;

/// Parse and lower a program on its own, and say where its names are
/// declared and used. The symbols are there if it parses, even with other
/// errors. It can't import: see [`compile_program`].
pub fn compile_with_symbols(src: &str) -> (Option<ir::Program>, Vec<Diagnostic>, Option<symbols::Symbols>) {
    let entry = Source {
        identity: String::new(),
        name: String::new(),
        text: src.to_string(),
    };
    let compiled = compile_program(entry, &mut NoImports);
    (compiled.program, compiled.diagnostics, compiled.symbols)
}

/// Parse and lower a program. The IR is only returned when there are no
/// errors; warnings are returned either way.
pub fn compile(src: &str) -> (Option<ir::Program>, Vec<Diagnostic>) {
    let (program, diags, _) = compile_with_symbols(src);
    (program, diags)
}
