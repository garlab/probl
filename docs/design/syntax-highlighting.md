# Shared syntax highlighting

> Implementation started 16 November 2026. The shared grammar and native
> consumer integrations are implemented. Consumer platform/rendering review, first
> publication and registry-only consumer builds remain release gates below.

## Implementation status

The independent [tree-sitter-probl package](../../editors/tree-sitter-probl/README.md)
contains the generated parser, bounded external scanner, Rust bindings, highlight
and fold queries, corpus, fixtures, licenses and node metadata. Generation is
pinned to Tree-sitter CLI 0.27.1. Rust 1.90 and Tree-sitter 0.27 are tested without
depending on the compiler. The [compiler comparison suite](../../editors/highlighting-tests/compiler.rs)
lives in a separate, unpublished workspace so it cannot raise the grammar's Rust
minimum or become a consumer dependency.

Plastext registers Probl files and case-insensitive fences, uses the shared
highlight/fold queries, and supplies a line-local fallback. Gloss registers the
same grammar for files, fences and `--language probl`, retaining its existing
limits and complete uncolored fallback. Both integrations have tests for token
colors, interpolation, UTF-8 boundaries and resource fallbacks. Gloss also tests
files/fences/stdin in both themes and source preservation under `NO_COLOR`,
`--no-highlight` and plain exports.

Local validation covers every example, benchmark and playground compiler fixture
(including CRLF), token boundaries against the compiler, operator precedence,
contextual words used as names, malformed input, structural folds, and incremental
edits/undo around strings, comments and brackets. Long comment runs cache newline
lookahead to avoid quadratic scanning. The [dedicated CI workflow](../../.github/workflows/syntax-highlighting.yml)
checks generated-file reproducibility, package builds, and the stable/minimum
Rust versions on macOS, Linux and Windows. All six hosted OS/toolchain combinations
and the generation/compiler checks [passed on 17 November 2026](https://github.com/garlab/probl/actions/runs/44033455944).
First publication and the consumer release checks below remain pending.

The grammar, packaged crate and both consumer test suites pass locally; the
Plastext macOS application also builds. Gloss's terminal tests cover both themes.
Plastext's visual review could not run because computer-use permissions were
unavailable. Native Linux/Windows consumer builds and representative rendering
review remain pending. Consumer manifests request registry version `0.1.0`;
untracked Cargo overrides currently resolve the unpublished crate locally.
After publication, remove those overrides, regenerate the consumer lockfiles
against the registry, and repeat the consumer builds before release.

The compiler currently rejects literal newlines in strings, including inside
interpolations. The grammar follows that rule: `\n` is an escape, while collections
and blocks can span lines. The original reference to multiline strings below has
been corrected; this work does not change the language or replace CodeMirror.

### Local performance measurements

Measured on an Apple M2 Pro with 16 GiB RAM, macOS 26.4.1 and Rust 1.99 release
builds. Gloss figures are medians of ten paired runs, alternating baseline and
modified binaries after two warmups. The baseline renders Probl without syntax
colors. Raw logs and generated inputs stay outside the repository.

| Gloss input | First output, before → after | Peak RSS, before → after |
|---|---:|---:|
| Small Probl with interpolation | 4.33 → 7.28 ms | 3.13 → 5.27 MiB |
| 4,500 reports, capture-limit fallback | 4.77 → 23.73 ms | 3.52 → 12.34 MiB |
| 1,000 unfinished interpolated strings | 4.62 → 23.78 ms | 3.34 → 6.13 MiB |
| Unchanged Rust sample | 15.96 → 15.84 ms | 7.72 → 7.69 MiB |

Gloss's release binary grows by 306.6 KiB and Plastext's by 323.3 KiB against
equivalent builds without Probl support. The shared query takes about 2.8 ms to
initialize. Plastext's syntax component parses/highlights the small Probl sample
in 14.1 µs after initialization; an edit plus undo and highlight takes 13.9 µs.
The unchanged Rust sample measures 6.7 µs for parse/highlight versus 7.0 µs in
the baseline, and 11.6 µs for edit/undo versus 11.3 µs.

The package's benchmark covers a 100 KB valid source (18.1 ms parse, 9.2 ms query,
3.6 ms edit/undo), a 100 KB comment run (4.7/1.3/7.6 ms), and 21 KB of malformed
strings (14.9/0.8/30.8 ms). These are local measurements rather than performance
thresholds. Native Plastext first-paint latency and process memory still need
measurement during the pending application review.

## Goal and approach

Highlight `.probl` files and `probl` Markdown code fences in native applications,
initially Plastext and Gloss. Maintain one reusable Tree-sitter grammar and its
highlight queries in this repository, next to the language they describe.

The existing [playground highlighter](../../web/src/probl-lang.js) provides a
useful starting point for token categories and examples. It is a CodeMirror
stream parser, so native Tree-sitter consumers cannot use it directly. Keep it
working while the native grammar is developed; replacing the playground parser
is a separate decision.

The [compiler lexer](../../crates/probl-syntax/src/lexer.rs),
[token definitions](../../crates/probl-syntax/src/token.rs), parser, and
[reference semantics](../semantics.md) define the syntax to follow. Tree-sitter
provides an incremental, error-tolerant syntax tree for presentation; it does
not replace the compiler or validate a model's meaning.

Using `probl-syntax` directly is a poor fit for this task: its API is internal,
it depends on numeric and diagnostic libraries, and it currently inherits Rust
1.99 from the workspace. Gloss supports Rust 1.90. Highlighting should identify
numeric tokens without constructing arbitrary-precision values and should never
run a model, resolve imports, read data, or access the network.

## Work in this repository

1. **Create one grammar package.** Use a directory such as
   `editors/tree-sitter-probl/`, with the proposed package name
   `tree-sitter-probl`. Keep it independent of the compiler crates and their
   release versions. Explicitly separate its Cargo workspace or exclude it from
   the parent workspace so it can declare its own minimum Rust version. Target
   compatibility with the consumers' Tree-sitter 0.27 and Rust 1.90; verify both.
2. **Implement the syntax.** Cover declarations, functions, imports and exports,
   annotations, collections, control flow, model tests, observations, reports,
   and expressions. Match precedence, contextual keywords, and newline handling
   to the compiler. Pay particular attention to dice (`d6`, `2d6`), percentages,
   binary/hexadecimal numbers, exponent notation, ranges and spread operators,
   single-quoted temporal literals, pragmas, comments, and strings
   with escapes and nested interpolated expressions. Recover locally from
   unfinished input during editing.
3. **Ship queries and Rust bindings.** Include a small `highlights.scm` using
   conventional capture names for keywords, types, functions, strings, escapes,
   numbers, constants, comments, properties, attributes, operators, and
   punctuation. Keep expressions inside interpolated strings distinguishable
   from literal text. Expose the grammar and highlight query as Rust constants,
   following the existing Tree-sitter binding conventions. Add fold queries for
   blocks and multiline constructs; symbol/tag queries can follow separately.
4. **Make builds reproducible.** Include the generated C parser, required
   headers, node-type metadata, queries, Rust bindings, and license files in the
   package. Pin and document the Tree-sitter generation tool. Consumers should
   need only their usual Rust/C toolchains, with no Node.js, generator, runtime
   download, or Probl installation. Add an external scanner only if the grammar
   needs one, with bounded state and incremental-reparse tests.
5. **Validate and publish the grammar.** Add the tests below, review the package
   contents, and verify a clean package build. Use the repository's MIT OR
   Apache-2.0 licensing and preserve any third-party notices. Check the proposed
   package name before the first release. Publish the grammar independently;
   consumers then depend on that registry version. Temporary local overrides
   belong in untracked Cargo configuration.

## Consumer integration

| Consumer | Required changes |
|---|---|
| Plastext | Register `.probl` and the case-insensitive `probl` fence label; connect the shared queries to its existing background/incremental highlighter and structural folds; add a small lexical fallback for files above the parse limit. |
| Gloss | Detect `.probl` as literal code; accept `probl` fences and `--language probl`; use the shared grammar and query with the existing lazy initialization, size/time/capture budgets, and complete uncolored fallback. |

Both consumers retain their own theme mapping, cancellation, and resource
limits. Keep syntax definitions and queries in the grammar package so language
changes do not require maintaining independent native highlighters. Highlighting
support does not require an LSP, formatter, compiler, or execution engine.

## Validation and completion criteria

- Add grammar corpus cases and highlight assertions for the language's token
  forms and constructs. Include UTF-8 text, CRLF, escaped braces and quotes,
  nested interpolation, ambiguous operators, and unfinished or malformed input.
- Parse valid programs from `examples/` and representative compiler fixtures;
  distinguish intentionally invalid fixtures from unexpected grammar errors.
  Compare token boundaries and precedence with the compiler, rather than
  treating Tree-sitter's error recovery as evidence that syntax is correct.
- Verify edits and undo near strings, comments, and block boundaries produce
  the same tree/highlights as a fresh parse. Check fold ranges and ensure
  highlight spans respect UTF-8 boundaries.
- In both apps, verify standalone files and fenced code. Test light/dark themes,
  bounded fallback on large inputs, and complete source preservation. In Gloss,
  also check stdin, plain output, `NO_COLOR`, and `--no-highlight`.
- Measure query initialization, first output or first paint, edit latency,
  memory, and binary size against an unchanged baseline. Include large and
  malformed input and confirm unrelated languages retain their behavior.
  Keep generated inputs and raw benchmark logs outside the repository.
- Verify native builds on macOS, Linux, and Windows, plus the grammar's declared
  minimum Rust version. Review representative rendering in both applications.

A language syntax change should update the compiler tests, grammar corpus and
queries, and playground rules together. The initial feature is complete when a
published grammar works in both consumers without local path dependencies.
