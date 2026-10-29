//! Union declarations and runtime descriptions agree in both execution modes.
mod common;
use common::*;
use probl_engine::{Options, value::Value};
use probl_sema::ir::Mode;

fn facts(source: &str) {
    for mode in [Mode::Enumerate, Mode::Sample { runs: 40, seed: 7 }] {
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
fn numeric_members_keep_then_widen_then_convert_to_probability() {
    facts(
        r#"
        let a: prob or float = 50%
        let b: prob or str = 50%
        let c: int or float = 1
        let d: float or prob = 1
        let e: int or prob = 1.0
        report typeof a == "float" and typeof b == "prob" and typeof c == "int"
        report typeof d == "float" and typeof e == "prob"
        let one = 1
        let reverse: prob or float = one
        let same: float or prob = one
        report typeof reverse == typeof same and typeof same == "float"
        fn take(x: prob or float): float or prob { x }
        let f = take
        report typeof take(1) == "float" and typeof f(1) == "float"
        report typeof f(prob(50%)) == "prob"
        fn outcome(): str or int { if 50% { return "a" } else { return 1 } }
        let result = outcome()
        report typeof result == "str" or typeof result == "int"
        let duplicate: int or (str or int) = "a"
        report duplicate == "a"
    "#,
    );
    for source in [
        "let x: int or str = 1.5",
        "let x: prob or str = 2",
        "let x: float or str = 9007199254740993",
    ] {
        assert!(!compile_error(source).is_empty());
    }
    assert!(error("let n = 1.5; let x: int or str = n; report x").contains("int or str"));
}

#[test]
fn container_parts_convert_independently_and_literals_choose_kind_first() {
    facts(
        r#"
        let a: list[float] or set[int] = [1, 1, 2]
        report typeof a == "list[float]" and len(a) == 3
        let b: str or set[int] = [1, 1, 2]
        report typeof b == "set[int]" and len(b) == 2
        let c: bag or map[int, float] = [1: 2]
        report typeof c == "map[int, float]"
        let d: str or bag = [1: 2]
        report typeof d == "bag[int]" and len(d) == 2
        let nested: list[set[int] or str] = [[1, 1], "a"]
        report typeof nested == "list[str or set[int]]"
        let mixed: list[int or float] = [1, 2.5]
        report typeof mixed[0] == "int" and typeof mixed[1] == "float"
        let original = [1, 2.5]
        let widened: str or list[float] = original
        report typeof widened == "list[float]" and typeof original == "list[int or float]"
        let duplicate: list[int or str] or list[str or int] = [1, "a"]
        report typeof duplicate == "list[int or str]"
        let branch: int or bag[int] = if 50% { [1, 1] } else { [2, 2] }
        report typeof branch == "bag[int]"
        fn size(xs: bag[int] or str) { len(xs) }
        report size([1, 1]) == 2
        let m: map[int or str, float or str] = [1: 2, "a": "b"]
        report typeof m == "map[int or str, float or str]"
    "#,
    );
    for literal in ["[1,1]", "[]"] {
        assert!(compile_error(&format!("let x: set[int] or bag[int] = {literal}")).contains("set or a bag"));
    }
    assert!(error("let xs=[1]; let x: set[int] or str=xs; report x").contains("set[int]"));
    assert!(error("let x: list[str] or bag[int] = [1]; report x").contains("list[str]"));
}

#[test]
fn every_union_position_rejects_ambiguous_kinds_and_record_shapes() {
    for pair in [
        ("map[int,float]", "map[float,int]"),
        ("list[int]", "list[str]"),
        ("set[int]", "set[float]"),
        ("bag", "bag[str]"),
        ("dist[int]", "dist[float]"),
        ("{a:float,b:prob}", "{a:prob,b:float}"),
    ] {
        for (a, b) in [pair, (pair.1, pair.0)] {
            for ty in [
                format!("{a} or {b}"),
                format!("list[{a} or {b}]"),
                format!("map[{a} or {b}, str]"),
                format!("map[str, {a} or {b}]"),
                format!("dist[{a} or {b}]"),
                format!("{{x: {a} or {b}}}"),
            ] {
                assert!(
                    compile_error(&format!("fn unused(x: {ty}) {{ x }}")).contains("in a union"),
                    "{ty}"
                );
            }
        }
    }
    assert!(compile_error("type A={x:int}; fn f(x:A or {x:float}) {x}").contains("same fields"));
    assert!(compile_error("type A={next:list[A], x:map[int,float] or map[float,int]}").contains("two map types"));
    facts(
        r#"
        type A = { x: int }
        type B = { x: int }
        let xs: list[A or B] = [A { x: 1 }, B { x: 2 }]
        report typeof xs == "list[A or B]"
        let disjoint: {x: float} or {y: str} = {x: 1}
        report typeof disjoint == "\{x: float\}"
        type Node = { next: Node or int }
        let node = Node { next: Node { next: 0 } }
        report typeof node == "Node"
        type Fields = { a: map[int,float], b: map[float,int] }
        report typeof Fields { a: [1:2], b: [1:2] } == "Fields"
    "#,
    );
    assert!(compile_error("type Loop = { next: Loop or { again: Loop } }").contains("forever"));
}

#[test]
fn typeof_merges_contents_structurally_and_in_a_stable_order() {
    facts(
        r#"
        report typeof [1,"a"] == "list[int or str]"
        report typeof [1,2.0] == "list[int or float]"
        report typeof [[],[1]] == "list[list[int]]"
        report typeof [[1],["a"]] == "list[list[int or str]]"
        report typeof [{x:1},{x:"a"}] == "list[\{x: int or str\}]"
        type Point = {x: int}
        type Vector = {x: str}
        report typeof [Point{x:1},{x:2}] == "list[\{x: int\}]"
        report typeof [Point{x:1}, Vector{x:"a"}, {x:2}] == "list[\{x: int or str\}]"
        report typeof [{x:2}, Vector{x:"a"}, Point{x:1}] == "list[\{x: int or str\}]"
        report typeof [Point{x:1}, Vector{x:"a"}] == "list[Point or Vector]"
        type Flexible = {x: int or str}
        let named = [Flexible{x:1}, Flexible{x:"a"}]
        report typeof named == "list[Flexible]"
        report typeof [...named, {x:2}] == "list[\{x: int or str\}]"
        report typeof [{x:2}, ...named] == "list[\{x: int or str\}]"
        report typeof [Flexible{x:1}, {x:2}, Flexible{x:"a"}] == "list[\{x: int or str\}]"
        report typeof one_of([1,"x"]) == "dist[int or str]"
        report typeof [set(),set(1),set("a")] == "list[set[int or str]]"
        report typeof [[:],[1:"a"],["a":1]] == "list[map[int or str, int or str]]"
        report typeof [d6,one_of(["a"])] == "list[dist[int or str]]"
        report typeof [true, "a", 1] == typeof [1, "a", true]
    "#,
    );
}

#[test]
fn unions_preserve_lazy_distribution_conversion_contracts() {
    facts(
        r#"
        let d: dist[int or float] = geometric(1e-16)
        report typeof d == "dist[int]"
        let f: dist[str or float] = geometric(1e-12)
        report typeof f == "dist[float]"
        let p: dist[str or prob] = binomial(1, 50%)
        report typeof p == "dist[prob]"
        let keep: dist[int or str] or bool = poisson(3)
        report typeof keep == "dist[int]" and pmf(keep, 2) == pmf(poisson(3), 2)
        let mixed: dist[int or str] = one_of([1,"a"])
        report typeof mixed == "dist[int or str]"
        let continuous: dist[int or float] = normal(0,1)
        report typeof continuous == "dist[float]"
        let vector: dist[str or list[int or float]] = multivariate_normal([0], [[1]])
        report typeof vector == "dist[list[float]]"
        let listed: dist[float or str] = poisson(3)
        report typeof listed == "dist[float]" and pmf(listed,2) == pmf(poisson(3),2)
    "#,
    );
    for source in [
        "let d: dist[float or str] = geometric(1e-16); report d",
        "let d: dist[prob or str] = poisson(1e-10); report d",
    ] {
        for mode in [Mode::Enumerate, Mode::Sample { runs: 1, seed: 7 }] {
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
fn mutation_and_assignment_keep_union_element_context() {
    facts(
        r#"
        var xs: list[set[int]] or int = [[1,1]]
        xs.push([2,2])
        xs[0] = [3,3]
        report xs == [set(3),set(2)]
        var values: list[int or str] = [1]
        values.push("a")
        report typeof values == "list[int or str]"
        var changing: list[int] or str = [1]
        changing = "a"
        report typeof changing == "str"
        changing = [2]
        report typeof changing == "list[int]"
        var m: map[str, set[int]] or bool = ["a": [1,1]]
        m.insert("b", [2,2])
        report m["b"] == set(2)
        type Box = { values: list[int or str] }
        var b: Box = Box { values: [1] }
        b.values.push("a")
        report typeof b.values == "list[int or str]"
        type Either = { value: list[float] or map[str, str] }
        var unannotated = Either { value: [1] }
        let one = 1
        unannotated.value[0] = one
        report typeof unannotated.value == "list[float]"
    "#,
    );
    assert!(compile_error("var xs: list[int or str] = [1]; xs.push(true); report xs").contains("int or str"));
    assert!(error("let b=true; var xs: list[int or str] = [1]; xs.push(b); report xs").contains("int or str"));
    assert!(!error("type E={x: list[int] or map[int,str]}; var e=E{x:[1]}; e.x[0]=true; report e").is_empty());
    assert!(compile_error("let xs: list[int or str] = read(\"a.json\")").contains("schemas don't support unions"));
}
