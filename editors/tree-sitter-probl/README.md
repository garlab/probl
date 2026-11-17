# Tree-sitter Probl

Shared syntax highlighting and structural folds for `.probl` files and `probl`
Markdown fences. The package contains its generated C parser: consumers need
only Rust and a C compiler. It never executes models, resolves imports or reads
data. It has no compiler dependency or runtime downloads.

```rust
let mut parser = tree_sitter::Parser::new();
parser.set_language(&tree_sitter_probl::LANGUAGE.into())?;
let query = tree_sitter::Query::new(
    &tree_sitter_probl::LANGUAGE.into(),
    tree_sitter_probl::HIGHLIGHTS_QUERY,
)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`LANGUAGE`, `HIGHLIGHTS_QUERY`, `FOLDS_QUERY` and `NODE_TYPES` are public.
Consumers provide their own theme mappings, parse/capture limits, cancellation
and fallback. Ignore fold captures that occupy a single line. Keep the first
matching highlight pattern when equal ranges overlap.

The grammar follows `crates/probl-syntax/src/{lexer,parser}.rs`. Contextual words
such as `as`, `to`, `export`, `test` and `expect` remain identifiers elsewhere.
String interpolations contain normal expression trees, with literal text and
escapes captured separately. Literal newlines in strings are invalid in the
compiler; use `\n`. Temporal tokens are highlighted without validating calendar
values or constructing numbers. This is a presentation parser, not a validator.

## Development

The package has its own Cargo workspace, release version and Rust 1.90 minimum.
The supported runtime is Tree-sitter 0.27. Regenerate with **tree-sitter-cli
0.27.1**, using Node.js to evaluate the grammar:

```sh
cargo install tree-sitter-cli --version 0.27.1 --locked --no-default-features
tree-sitter generate --js-runtime node
tree-sitter test
cargo +1.90.0 test --locked
cargo package --locked --allow-dirty
cargo bench --bench highlighting
```

Commit the generated parser, headers, grammar JSON and node types with grammar
changes. Consumers do not run the generator. The small external scanner tracks
brackets and quotes so newlines inside parentheses and lists remain whitespace
while nested blocks retain statement boundaries and strings remain single-line.
Its entire serialized state is bounded to 263 bytes and
serialized for incremental parsing; nesting beyond 256 recovers as malformed
input. The compiler itself limits nesting to 200.

When changing language syntax, update compiler tests, this corpus and highlight
assertions, and the playground stream parser together. Rust tests also parse the
repository examples, benchmarks and playground fixtures when run in a checkout.
Run `cargo test --manifest-path ../highlighting-tests/Cargo.toml --locked` with
the compiler's Rust toolchain for token-boundary comparisons against its lexer.
That separate workspace is deliberately excluded from the published package.

## Release

Release this crate independently of the compiler. Check the registry name and
review `cargo package --list` before publishing. Use registry versions in
consumers; temporary path overrides belong in untracked Cargo configuration.
The repository's MIT OR Apache-2.0 licenses apply; `NOTICE` identifies the
Tree-sitter header license.
