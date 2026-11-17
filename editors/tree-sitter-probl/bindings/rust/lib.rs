//! Probl syntax for Tree-sitter. Parsing never executes a model or reads imports.
//!
//! ```
//! let mut parser = tree_sitter::Parser::new();
//! parser.set_language(&tree_sitter_probl::LANGUAGE.into()).unwrap();
//! let tree = parser.parse("let roll ~ 2d6\nreport roll >= 7", None).unwrap();
//! assert!(!tree.root_node().has_error());
//! ```

use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_probl() -> *const ();
}

/// Generated grammar, compatible with Tree-sitter 0.27.
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_probl) };
/// Conventional highlight captures; expression parts of strings remain code.
pub const HIGHLIGHTS_QUERY: &str = include_str!("../../queries/highlights.scm");
/// Structural folds. Consumers should discard captures that occupy one line.
pub const FOLDS_QUERY: &str = include_str!("../../queries/folds.scm");
/// Generated syntax node metadata.
pub const NODE_TYPES: &str = include_str!("../../src/node-types.json");
