use std::{fs, path::Path};
use tree_sitter::{InputEdit, Node, Parser, Point, Query, QueryCursor, StreamingIterator, Tree};
use tree_sitter_probl::{FOLDS_QUERY, HIGHLIGHTS_QUERY, LANGUAGE};

fn parser() -> Parser {
    let mut parser = Parser::new();
    parser.set_language(&LANGUAGE.into()).unwrap();
    parser
}

fn parse(source: &str) -> Tree {
    let tree = parser().parse(source, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{source}\n{}",
        tree.root_node().to_sexp()
    );
    tree
}

fn captures(source: &str, tree: &Tree, query: &str) -> Vec<(usize, usize, String)> {
    let query = Query::new(&LANGUAGE.into(), query).unwrap();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.captures(&query, tree.root_node(), source.as_bytes());
    let mut spans = Vec::new();
    while let Some((found, index)) = matches.next() {
        let capture = found.captures()[*index];
        let range = capture.node.byte_range();
        assert!(source.is_char_boundary(range.start) && source.is_char_boundary(range.end));
        spans.push((
            range.start,
            range.end,
            query.capture_names()[capture.index as usize].to_owned(),
        ));
    }
    spans.sort();
    spans
}

#[test]
fn queries_compile() {
    Query::new(&LANGUAGE.into(), HIGHLIGHTS_QUERY).unwrap();
    Query::new(&LANGUAGE.into(), FOLDS_QUERY).unwrap();
}

#[test]
fn fixtures_parse() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("test/fixtures");
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "probl") {
            let source = fs::read_to_string(&path).unwrap();
            eprintln!("{}", path.display());
            parse(&source);
            parse(&source.replace('\n', "\r\n"));
        }
    }
}

#[test]
fn repository_examples_and_playground_fixtures() {
    // Published packages carry the standalone fixtures above. The checkout also
    // exercises every example, benchmark and playground compiler fixture.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("crates/probl-syntax").exists() {
        return;
    }
    for dir in ["examples", "benches", "web/test"] {
        for entry in fs::read_dir(root.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "probl") {
                eprintln!("{}", path.display());
                parse(&fs::read_to_string(path).unwrap());
            }
        }
    }
}

fn expression_shape(node: Node, source: &str) -> String {
    match node.kind() {
        "binary_expression" => format!(
            "({} {} {})",
            node.child_by_field_name("operator")
                .unwrap()
                .utf8_text(source.as_bytes())
                .unwrap(),
            expression_shape(node.child_by_field_name("left").unwrap(), source),
            expression_shape(node.child_by_field_name("right").unwrap(), source)
        ),
        "unary_expression" => format!(
            "({} {})",
            node.child_by_field_name("operator")
                .unwrap()
                .utf8_text(source.as_bytes())
                .unwrap(),
            expression_shape(node.child_by_field_name("operand").unwrap(), source)
        ),
        "parenthesized_expression" => expression_shape(node.named_child(0).unwrap(), source),
        _ => node.utf8_text(source.as_bytes()).unwrap().to_owned(),
    }
}

#[test]
fn compiler_binding_powers() {
    for (source, expected) in [
        ("a or b and not c == d", "(or a (and b (not (== c d))))"),
        ("-x ^ 2 * 3 + 4", "(+ (* (- (^ x 2)) 3) 4)"),
        ("2 ^ 3 ^ 2", "(^ 2 (^ 3 2))"),
        ("typeof x ^ 2", "(^ (typeof x) 2)"),
        ("~d6 ^ 2", "(^ (~ d6) 2)"),
        ("a..<n - 1", "(..< a (- n 1))"),
        ("3% to 7%", "(to 3% 7%)"),
        (
            "x not in [1, 2] and y in 1..6",
            "(and (not in x [1, 2]) (in y (.. 1 6)))",
        ),
    ] {
        let tree = parse(source);
        let node = tree.root_node().named_child(0).unwrap().named_child(0).unwrap();
        assert_eq!(expression_shape(node, source), expected, "{source}");
    }
}

#[test]
fn comparisons_and_ranges_require_parentheses_to_chain() {
    for source in ["a < b < c", "a == b in c", "1..2..3", "1 to 2..<3"] {
        assert!(
            parser().parse(source, None).unwrap().root_node().has_error(),
            "{source}"
        );
    }
    for source in ["(a < b) == c", "a < (b < c)", "(1..2)..3", "a < b and b < c"] {
        parse(source);
    }
}

#[test]
fn newlines_follow_the_compiler_lexer() {
    for source in [
        "let a = 1 +\n 2\nreport a",
        "let a = (1\n + 2)\nreport a",
        "let a = f(\n { let x = 1\n x }\n)\nreport a",
        "return\n1",
        "let x = 1\n# chain\n .abs()\nreport x",
        "let x = [1,\n2]\nreport x",
        "if x { y }\nelse { z }\nreport x",
        "try { x }\ncatch Fault { y }\nreport x",
        "chance { 50% => if x { 1 }\nelse => 2 }",
        "if x { a }\nif y { b }",
    ] {
        parse(source);
    }
    for source in [
        "let\nx = 1",
        "let x = 1\n+ 2",
        "report 1 to\n2",
        "let x = 1\n..2",
        "let x = 1 let y = 2",
    ] {
        assert!(
            parser().parse(source, None).unwrap().root_node().has_error(),
            "{source}"
        );
    }
    let tree = parse("return\n1");
    assert_eq!(tree.root_node().named_child_count(), 2);
}

#[test]
fn reserved_words_and_dice_cannot_be_names() {
    for source in [
        "let if = 1",
        "fn return() {}",
        "report _",
        "let d6 = 1",
        "let x = 2d6_",
        "let x = d6x",
    ] {
        let tree = parser().parse(source, None).unwrap();
        assert_eq!(tree.root_node().has_error(), source != "let x = d6x", "{source}");
    }
}

#[test]
fn highlights_cover_tokens_without_coloring_interpolation_as_text() {
    let source = "@mode enumerate\nfn roll(x: int): int { x }\nlet to = 2d6\nlet as = d6_0\nlet s = \"é 🎲 \\{ {roll(to) + 1} \\\" {\"nested {2}\"}\"\nreport [0b10_01, 0Xff, 1_000.5e-2%, d6, '1h 30m', true] # UTF-8 λ\n";
    let tree = parse(source);
    let spans = captures(source, &tree, HIGHLIGHTS_QUERY);
    for (text, kind) in [
        ("mode", "attribute"),
        ("roll", "function"),
        ("int", "type"),
        ("2d6", "number"),
        ("d6", "number"),
        ("0b10_01", "number"),
        ("0Xff", "number"),
        ("1_000.5e-2%", "number"),
        ("'1h 30m'", "constant"),
        ("true", "constant.builtin"),
        ("\\{", "string.escape"),
        ("# UTF-8 λ", "comment"),
        ("d6_0", "variable"),
    ] {
        assert!(
            spans.iter().any(|(a, b, k)| &source[*a..*b] == text && k == kind),
            "missing {kind} {text:?}: {spans:?}"
        );
    }
    for (a, b, kind) in &spans {
        if kind == "string" {
            assert!(!source[*a..*b].contains("roll(to)"));
        }
        if kind == "keyword" || kind == "operator" {
            assert_ne!(&source[*a..*b], "to");
            assert_ne!(&source[*a..*b], "as");
        }
    }
}

fn point(source: &str, byte: usize) -> Point {
    Point {
        row: source[..byte].bytes().filter(|&b| b == b'\n').count(),
        column: byte - source[..byte].rfind('\n').map_or(0, |p| p + 1),
    }
}

fn compare_incremental(before: &str, start: usize, end: usize, replacement: &str) {
    let mut parser = parser();
    let mut old = parser.parse(before, None).unwrap();
    let after = format!("{}{}{}", &before[..start], replacement, &before[end..]);
    old.edit(&InputEdit {
        start_byte: start,
        old_end_byte: end,
        new_end_byte: start + replacement.len(),
        start_position: point(before, start),
        old_end_position: point(before, end),
        new_end_position: point(&after, start + replacement.len()),
    });
    let incremental = parser.parse(&after, Some(&old)).unwrap();
    let fresh = parser.parse(&after, None).unwrap();
    assert_eq!(
        incremental.root_node().to_sexp(),
        fresh.root_node().to_sexp(),
        "{after}"
    );
    for query in [HIGHLIGHTS_QUERY, FOLDS_QUERY] {
        assert_eq!(
            captures(&after, &incremental, query),
            captures(&after, &fresh, query),
            "{after}"
        );
    }
    // Undo the edit with the same parser and edited tree.
    let mut edited = incremental;
    edited.edit(&InputEdit {
        start_byte: start,
        old_end_byte: start + replacement.len(),
        new_end_byte: end,
        start_position: point(&after, start),
        old_end_position: point(&after, start + replacement.len()),
        new_end_position: point(before, end),
    });
    let undone = parser.parse(before, Some(&edited)).unwrap();
    let original = parser.parse(before, None).unwrap();
    assert_eq!(undone.root_node().to_sexp(), original.root_node().to_sexp());
    assert_eq!(
        captures(before, &undone, HIGHLIGHTS_QUERY),
        captures(before, &original, HIGHLIGHTS_QUERY)
    );
}

#[test]
fn edits_and_undo_match_a_fresh_parse() {
    let source = "fn f(x) {\n # é 🎲 comment\n let a = [\n { let y = x\n y }, 2\n ]\n report \"λ {f(\"nested {x}\")} \\{\"\n}\nreport f(1)\n";
    parse(source);
    for (offset, character) in source.char_indices() {
        compare_incremental(source, offset, offset + character.len_utf8(), "");
        if "\n#{}[]()\"\\".contains(character) {
            compare_incremental(source, offset, offset, "\n");
        }
    }
}

#[test]
fn edits_in_a_comment_run_invalidate_newline_lookahead() {
    let source = "report x\n# first\n\n# last\n .abs()\nreport 2\n";
    parse(source);
    for (offset, character) in source.char_indices() {
        compare_incremental(source, offset, offset + character.len_utf8(), "");
        compare_incremental(source, offset, offset, "x");
    }
    parse(&format!("report x\n{} .abs()", "# comment\n".repeat(20_000)));
}

#[test]
fn folds_are_structural() {
    let source = "fn f() {\n let x = [\n 1, 2\n ]\n report x\n}\n";
    let spans = captures(source, &parse(source), FOLDS_QUERY);
    assert_eq!(spans.len(), 3); // parameters, block, list; consumers omit one-line folds
    assert!(spans.iter().any(|(a, b, _)| &source[*a..*b] == "[\n 1, 2\n ]"));
    assert!(spans.iter().all(|(a, b, _)| *a > 0 && *b < source.len()));
}

#[test]
fn malformed_input_recovers_and_stays_utf8_safe() {
    for source in [
        "let x = \"unfinished\nreport 2\n",
        "let x = 'unfinished\nreport 2\n",
        "let x = \"bad \\q\"\nreport 2",
        "let x = [1,\nreport 2",
        "if x {\nreport 2",
        "let x = 0b102\nreport 2",
        "let x = 0xffg\nreport 2",
        "let x = 2d6x\nreport 2",
        "let x = \"é {1 + }\"\nreport 2",
        "let x = 😀\nreport 2",
        "report [\"first\nsecond\"]\nreport 2",
        "report \"{(\n1)}\"\nreport 2",
    ] {
        let tree = parser().parse(source, None).unwrap();
        assert!(tree.root_node().has_error(), "{source}");
        let spans = captures(source, &tree, HIGHLIGHTS_QUERY);
        assert!(
            spans
                .iter()
                .any(|(a, b, k)| &source[*a..*b] == "report" && k == "keyword"),
            "{source}"
        );
    }
}
