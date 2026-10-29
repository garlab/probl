//! Collection construction preserves values and evaluation order in both modes.
mod common;

use common::*;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Limits, Options};
use probl_sema::ir::Mode;

fn modes() -> [Mode; 2] {
    [Mode::Enumerate, Mode::Sample { runs: 40, seed: 7 }]
}

fn facts(source: &str) {
    for mode in modes() {
        let out = exec_raw(
            source,
            &Options {
                mode: Some(mode),
                ..Options::default()
            },
        )
        .unwrap_or_else(|e| panic!("{source}\n{e:?}"));
        assert!(!out.reports.is_empty());
        for report in out.reports {
            assert_eq!(report.distribution(), vec![(Value::Bool(true), 1.0)], "{source}");
        }
    }
}

#[test]
fn sets_compare_by_value_but_worlds_keep_representative_types() {
    facts(
        r#"
        let s = set(3, 1, 1.0, 2)
        report typeof s == "set[int]" and len(s) == 3 and s.contains(1.0) and 2 in s
        report list(...s) == [1, 2, 3] and set(1, 2) == set(2.0, 1.0)
        report typeof set(1.0, 1) == "set[float]"
        report set(...[[1], [1.0]]) == set(...[[1]])
        let m = [set(1): "found"]
        report m[set(1.0)] == "found"
        let empty: set[int] = []
        report typeof empty == "set[unknown]" and len(set()) == 0
        var total = 0
        for x in s { total += x }
        report total == 6
        let x = if 50% { set(1) } else { set(1.0) }
        report (typeof x == "set[int]") or (typeof x == "set[float]")
    "#,
    );
    let dist = distribution("let x = if 50% { set(1) } else { set(1.0) }; report typeof x");
    assert_eq!(dist.len(), 2);
    assert!(dist.iter().all(|(_, weight)| *weight == 0.5));
}

#[test]
fn typed_literals_and_bare_bags_keep_their_contracts() {
    facts(
        r#"
        let a = bag(1, 1, 2, 3)
        let b: bag = [1, 1, 2, 3]
        let c: bag = [1: 2, 2: 1, 3: 1]
        report a == b and b == c
        let mixed: bag = [1, "A"]
        report mixed.get(1) == 1 and mixed.get("A") == 1
        let none: bag = [:]
        report len(none) == 0 and len(bag()) == 0
        let huge: bag = [1: 10^12]
        report len(huge) == 10^12
        let xs = [1, 2]
        let floats: set[float] = [...xs, 2, 3]
        report typeof floats == "set[float]" and len(floats) == 3
        let nested: list[set[int]] = [[1, 1], [2]]
        report nested == [set(1), set(2)]
        fn make(): set[int] { if 50% { [1, 1] } else { [1] } }
        fn size(s: set[int]) { len(s) }
        report make() == set(1) and size([1, 1, 2]) == 2
        type Box = { items: set[int], bagged: bag }
        let box = Box { items: [1, 1], bagged: [2: 3] }
        report box.items == set(1) and box.bagged == bag(2, 2, 2)
        var changing: bag = []
        changing = ["A": 2]
        report changing == bag("A", "A")
    "#,
    );
    for src in [
        "var b: bag = []; let xs=[1]; b=xs",
        "let m=[1:2]; let b: bag=m",
        "let s: set[int] = list(1,2)",
        "fn f(s: set[int]) { len(s) }; let g=f; g([1,2])",
        "let x=1.5; let s: set[int] = set(x)",
        "let b: bag = [1: -1]",
    ] {
        for mode in modes() {
            assert!(
                exec_raw(
                    src,
                    &Options {
                        mode: Some(mode),
                        ..Options::default()
                    }
                )
                .is_err(),
                "{src}"
            );
        }
    }
    assert!(compile_error("let b: bag = [1: 2.0]").contains("int"));
}

#[test]
fn constructors_preserve_recipes_and_spreads_use_iteration() {
    facts(
        r#"
        report typeof list(d6, 0) == "list[int or dist[int]]"
        report typeof bag(d6) == "bag[dist[int]]"
        report typeof set(d6, d6) == "set[dist[int]]" and len(set(d6, d6)) == 1
        report typeof bag(geometric(1e-12)) == "bag[dist[int]]"
        report typeof list(normal(0,1)) == "list[dist[float]]"
        report list(..."é🦀") == ["é", "🦀"]
        report list(...[1: "a", 2: "b"]) == [[1, "a"], [2, "b"]]
        report bag(...[1: 2]).get([1, 2]) == 1
        let b: bag = [1: 2, 2: 1]
        report list(...b) == [1, 1, 2]
        report [...1..3, ...0..<2] == [1, 2, 3, 0, 1]
        report list(...[[1]]) == [[1]] and len(set(...[[1]])) == 1
        report list() == [] and list(...[]) == []
        let make = list
        let high = max
        report make(...[1,2]) == [1,2] and high(...[1,2]) == 2
        report pmf(max(...[d2, 1]), 2) == 50%
    "#,
    );
}

#[test]
fn spreads_run_once_in_order_across_mutations_and_world_splits() {
    facts(
        r#"
        var xs = [1, 2]
        var calls = 0
        let result = [...xs, { xs.push(3); calls += 1; 0 }, ...xs]
        report result == [1, 2, 0, 1, 2, 3] and calls == 1
        let before = list(...xs, { xs = [9]; 0 }, ...xs)
        report before == [1, 2, 3, 0, 9]
        let items = if 50% { [1] } else { [1, 2, 3] }
        report list(...items) == items and max(...items, 4) == 4
        fn outer(x) { let captured = y -> x + y; list(...[captured(0)], 2) }
        report outer(7) == [7, 2]
    "#,
    );
    // Expansion fails before evaluating a later hoisted block.
    let (program, diagnostics) = probl_sema::compile("report list(...d6, { print(\"too late\"); 0 })");
    assert!(diagnostics.is_empty());
    let mut printed = Vec::new();
    let result = probl_engine::run(&program.unwrap(), &Options::default(), &mut |line| {
        printed.push(line.to_owned())
    });
    assert!(result.is_err());
    assert!(printed.is_empty());
}

#[test]
fn spread_errors_and_transition_restrictions_are_explicit() {
    for source in [
        "report list([1])",
        "report set([1])",
        "report bag([1])",
        "let f=bag; report f([1])",
    ] {
        assert!(error(source).contains("single collection"), "{source}");
    }
    for source in [
        "report [...d6]",
        "report set(...normal(0,1))",
        "report bag(...geometric(1e-12))",
    ] {
        assert!(error(source).contains("spread a distribution"), "{source}");
    }
    assert!(error("report max(...[])").contains("at least 2"));
    assert!(compile_error("abs(...[1])").contains("spreading"));
    assert!(compile_error("fn f(x) { x }; f(...[1])").contains("spreading"));
    assert!(error("fn f(x) { x }; let g=f; report g(...[1])").contains("spreading"));
    assert!(error("let f=abs; report f(...[1])").contains("spreading"));
    assert!(compile_error("bag(counts: [1:2])").contains("named argument"));
}

#[test]
fn expansion_respects_collection_limits() {
    let opts = Options {
        limits: Limits {
            max_collection: 8,
            ..Limits::default()
        },
        ..Options::default()
    };
    for src in [
        "report [...1..100]",
        "let b: bag = [1: 10^12]; report list(...b)",
        "report list(...1..5, ...1..5)",
    ] {
        let err = exec_raw(src, &opts).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Limit, "{src}: {err:?}");
    }
}

#[test]
fn printing_keeps_counts_compact_and_set_source_unambiguous() {
    assert_eq!(distribution("report set(...[[1]])")[0].0.to_string(), "set(...[[1]])");
    assert_eq!(distribution("report set(3, 1, 1, 2)")[0].0.to_string(), "set(1, 2, 3)");
    assert_eq!(
        distribution("let b: bag = [1: 10^12]; report b")[0].0.to_string(),
        "bag [1: 1000000000000]"
    );
    assert_eq!(
        distribution("report [bag(1, 1, 2), bag()]")[0].0.to_string(),
        "[bag [1: 2, 2: 1], bag []]"
    );
    assert!(
        distribution("report bag(d6, d6)")[0]
            .0
            .to_string()
            .starts_with("bag [dist(")
    );
    for src in ["set()", "set(1, 2, 3)", "set(...[[1]])", "set(...[set(1)])"] {
        let value = distribution(&format!("report {src}"))[0].0.clone();
        let printed = value.to_string();
        let reparsed = distribution(&format!("report {printed}"))[0].0.clone();
        assert!(probl_engine::value::same(&value, &reparsed), "{printed}");
    }
}

#[test]
fn bare_collections_infer_types_from_contents() {
    facts(
        r#"
        let x: set = [1, 2, 1]
        report x == set(1, 2) and typeof x == "set[int]"
        let xs: list = [1, 2, 1]
        let m: map = [1: "a", 2: "b"]
        let b: bag = [1, 2, 1]
        report typeof xs == "list[int]" and len(xs) == 3
        report typeof m == "map[int, str]" and m[1] == "a"
        report typeof b == "bag[int]" and b.get(1) == 2
        let nested: set = [[1, 2], [1, 2]]
        report typeof nested == "set[list[int]]" and len(nested) == 1
        let mixed: list = [1, 2.0]
        report typeof mixed[0] == "int" and typeof mixed[1] == "float"
        let first: set = [1.0, 1]
        report typeof first == "set[float]"
        let empty_set: set = []
        let empty_list: list = []
        let empty_map: map = [:]
        report typeof empty_set == "set[unknown]" and typeof empty_list == "list[unknown]"
        report typeof empty_map == "map[unknown, unknown]"
        let range: list = 1..3
        report typeof range == "range" and len(range) == 3
        let recipe: list = [d6]
        report typeof recipe == "list[dist[int]]"
    "#,
    );
}

#[test]
fn bare_kinds_reach_branches_functions_records_and_nested_literals() {
    facts(
        r#"
        let chosen: set = if 50% { [1, 1] } else { [1] }
        let matched: set = match 1 { 1 => [1, 1], _ => [1] }
        let weighted: set = chance { 50% => [1, 1], else => [1] }
        let blocked: set = { let xs = [1, 1]; [...xs] }
        report chosen == set(1) and matched == chosen and weighted == chosen and blocked == chosen
        fn size(s: set) { len(s) }
        fn made(): set { if 50% { [1, 1] } else { return [1] } }
        fn copied(xs: list): list { xs }
        fn mapping(): map { [1: "a"] }
        report size([1, 1, 2]) == 2 and typeof made() == "set[int]"
        report typeof copied([1]) == "list[int]" and typeof mapping() == "map[int, str]"
        let nested: list[set] = [[1, 1], [2, 2]]
        let keyed: map[set, list] = [[1, 1]: [2, 3]]
        report nested == [set(1), set(2)] and typeof nested == "list[set[int]]"
        report keyed[set(1)] == [2, 3] and typeof keyed == "map[set[int], list[int]]"
        type Box = { s: set, xs: list, m: map }
        var box: Box = Box { s: [1, 1], xs: [2], m: [3: "a"] }
        box.s = [2, 2]
        let replaced = box with { s: [3, 3] }
        report box.s == set(2) and replaced.s == set(3)
        let anonymous: { s: set } = { s: [1, 1] }
        report anonymous.s == set(1)
        var changed: list[set] = [[1, 1]]
        changed[0] = [2, 2]
        changed.push([3, 3])
        report changed == [set(2), set(3)]
    "#,
    );
}

#[test]
fn bare_annotations_keep_the_kind_without_freezing_element_types() {
    facts(
        r#"
        var s: set = [1, 1]
        s = ["a", "a"]
        report typeof s == "set[str]"
        var xs: list = []
        xs.push(1)
        report typeof xs == "list[int]"
        xs[0] = "a"
        report typeof xs == "list[str]"
        var m: map = [1: 2]
        m[1] = "a"
        report typeof m == "map[int, str]"
        let existing = set(1, 2)
        let same: set = existing
        let converted: set[float] = same
        report typeof same == "set[int]" and typeof converted == "set[float]"
        let varying: set = if 50% { [1] } else { ["a"] }
        report typeof varying == "set[int]" or typeof varying == "set[str]"
    "#,
    );
    for source in [
        "let xs = [1, 2]; let s: set = xs; report s",
        "var s: set = [1]; s = list(2); report s",
        "var xs: list = []; xs = set(1); report xs",
        "var m: map = [:]; m = [1]; report m",
        "let s: set = [1: 2]; report s",
        "let bad = \"a\"; let s: set[int] = [bad]; report s",
    ] {
        for mode in modes() {
            assert!(
                exec_raw(
                    source,
                    &Options {
                        mode: Some(mode),
                        ..Options::default()
                    }
                )
                .is_err(),
                "{source}"
            );
        }
    }
}

#[test]
fn bare_collection_schemas_still_need_explicit_element_types() {
    for ty in ["list", "map", "set", "bag", "list[set]"] {
        assert!(compile_error(&format!("let xs: {ty} = read(\"a.json\")")).contains("needs type arguments"));
    }
    for ty in ["list[int, str]", "set[int, str]", "bag[int, str]", "map[int]"] {
        assert!(compile_error(&format!("let xs: {ty} = []")).contains("type argument"));
    }
}
