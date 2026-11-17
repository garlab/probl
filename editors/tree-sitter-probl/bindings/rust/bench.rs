//! Small reproducible timing harness; keep generated inputs and raw output out
//! of version control. Run `cargo bench --bench highlighting` in this package.
use std::{hint::black_box, time::Instant};
use tree_sitter::{InputEdit, Parser, Point, Query, QueryCursor, StreamingIterator};
use tree_sitter_probl::{HIGHLIGHTS_QUERY, LANGUAGE};

fn main() {
    let language = LANGUAGE.into();
    let start = Instant::now();
    for _ in 0..100 {
        black_box(Query::new(&language, HIGHLIGHTS_QUERY).unwrap());
    }
    println!(
        "query initialization: {:.1} us",
        start.elapsed().as_micros() as f64 / 100.0
    );
    let query = Query::new(&language, HIGHLIGHTS_QUERY).unwrap();
    for (name, source) in [
        ("small", "let x ~ 2d6\nreport \"café {x + 1}\"\n".to_owned()),
        ("large", "let x ~ 2d6\nreport x > 3\n".repeat(4000)),
        (
            "comments",
            format!("report x\n{}.abs()\n", "# comment\n".repeat(10_000)),
        ),
        ("malformed", "report \"unfinished {\n".repeat(1000)),
    ] {
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let start = Instant::now();
        let mut tree = parser.parse(&source, None).unwrap();
        let parse_us = start.elapsed().as_micros();
        let start = Instant::now();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.captures(&query, tree.root_node(), source.as_bytes());
        let mut count = 0;
        while matches.next().is_some() {
            count += 1;
        }
        let query_us = start.elapsed().as_micros();
        // Insertion and undo at the start of the file, retaining the old tree.
        let after = format!("# edit\n{source}");
        tree.edit(&InputEdit {
            start_byte: 0,
            old_end_byte: 0,
            new_end_byte: 7,
            start_position: Point::new(0, 0),
            old_end_position: Point::new(0, 0),
            new_end_position: Point::new(1, 0),
        });
        let start = Instant::now();
        let mut edited = parser.parse(&after, Some(&tree)).unwrap();
        edited.edit(&InputEdit {
            start_byte: 0,
            old_end_byte: 7,
            new_end_byte: 0,
            start_position: Point::new(0, 0),
            old_end_position: Point::new(1, 0),
            new_end_position: Point::new(0, 0),
        });
        black_box(parser.parse(&source, Some(&edited)).unwrap());
        println!(
            "{name}: {} bytes; parse {parse_us} us; captures {query_us} us ({count}); edit+undo {} us",
            source.len(),
            start.elapsed().as_micros()
        );
    }
}
