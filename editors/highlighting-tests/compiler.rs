//! Cross-check presentation syntax with the real compiler without coupling the
//! published grammar or its Rust minimum to the compiler's dependencies.
use probl_syntax::{lexer, parser::parse_tests_at, token::Tok};
use std::{fs, path::Path};
use tree_sitter::{Node, Parser};

fn tokens(node: Node, output: &mut Vec<(usize, usize)>) {
    if node.kind() == "comment" {
        return;
    }
    if node.kind() == "string" || node.child_count() == 0 {
        if node.end_byte() > node.start_byte() {
            output.push((node.start_byte(), node.end_byte()));
        }
    } else {
        for child in node.children(&mut node.walk()) {
            tokens(child, output);
        }
    }
}

fn check(source: &str) {
    let (_, _, diagnostics) = parse_tests_at(source, 0);
    assert!(
        diagnostics.is_empty(),
        "compiler rejects fixture: {diagnostics:?}\n{source}"
    );
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_probl::LANGUAGE.into()).unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "grammar rejects fixture:\n{source}\n{}",
        tree.root_node().to_sexp()
    );
    let expected: Vec<_> = lexer::lex(source, 0)
        .0
        .into_iter()
        .filter(|t| !matches!(t.tok, Tok::Newline | Tok::Eof))
        .map(|t| (t.span.lo as usize, t.span.hi as usize))
        .collect();
    let mut actual = Vec::new();
    tokens(tree.root_node(), &mut actual);
    assert_eq!(actual, expected, "token boundaries differ:\n{source}");
}

#[test]
fn valid_compiler_sources_have_identical_token_boundaries() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for dir in [
        "examples",
        "benches",
        "web/test",
        "editors/tree-sitter-probl/test/fixtures",
    ] {
        for entry in fs::read_dir(root.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "probl") {
                eprintln!("{}", path.display());
                let source = fs::read_to_string(path).unwrap();
                check(&source);
                check(&source.replace('\n', "\r\n"));
            }
        }
    }
}

#[test]
fn contextual_words_and_ambiguous_lexemes() {
    for source in [
        "export(1); test(2); export + test; report to; report as; report by; report from; expect(3)",
        "let x = [to, as, by, from, export, test, expect]; report x.test(export: 1)",
        "let from = 1; let to = 2; let as = 3; let by = 4; let export = 5; let test = 6; let expect = 7",
        "report [d6, 2d6, 2d6_0, d6_0, d6x, 0x1e2, 0b10_10, 1.2e-3%, 1..2, 1..<2]",
        "report [1, ...xs]; report f(...xs); report 1 not in [2]",
        "report \"{f(\"inner {x}\")} \\{ \\} 🎲\"",
        "let x = (1\n+2)\nreport x\n # a comment\n .abs()",
        "report f({ let a = 1\n a },\n2)",
    ] {
        check(source);
    }
}

#[test]
fn malformed_fixtures_are_explicitly_invalid() {
    // Deliberately invalid syntax is never included in the valid-program loop.
    // Calendar/range/meaning errors are the compiler's responsibility.
    for source in [
        "report a < b < c",
        "report 1..2..3",
        "report 1 to 2..<3",
        "let x = 0b102",
        "let x = 2d6x",
        "let if = 1",
        "report \"open\nreport 2",
        "let x = 1\n+2",
        "let x = [1,",
        "if x {",
    ] {
        assert!(!parse_tests_at(source, 0).2.is_empty(), "{source}");
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_probl::LANGUAGE.into()).unwrap();
        assert!(parser.parse(source, None).unwrap().root_node().has_error(), "{source}");
    }
}
