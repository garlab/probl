//! Runtime inspection before changing the probability/distribution semantics.
mod common;

use common::*;
use probl_engine::value::Value;
use probl_engine::{Limits, Options};
use probl_sema::ir::Mode;

fn type_of(expression: &str) -> String {
    let values = distribution(&format!("report typeof ({expression})"));
    assert_eq!(values.len(), 1);
    close(values[0].1, 1.0);
    let Value::Str(name) = &values[0].0 else {
        panic!("typeof must return a string")
    };
    name.to_string()
}

#[test]
fn scalar_types_expose_current_percentage_rules() {
    for (expr, expected) in [
        ("true", "bool"),
        ("33", "int"),
        ("2^100", "int"),
        ("0.33", "float"),
        ("33%", "float"),
        ("0%", "float"),
        ("100%", "float"),
        ("150%", "float"),
        ("-33%", "float"),
        ("33% + 1%", "float"),
        ("33% * 2", "float"),
        ("33% == 0.33", "bool"),
        ("pi", "float"),
        ("complex(0, 1)", "complex"),
        ("\"hello\"", "str"),
        ("date(2026, 10, 2)", "date"),
        ("1..10", "range"),
        ("{}", "()"),
        ("(x -> x)", "fn"),
        ("P(d6 > 3)", "prob"),
        ("mean(d6)", "float"),
    ] {
        assert_eq!(type_of(expr), expected, "{expr}");
    }
    // A literal in an annotated probability binding is converted.
    assert_eq!(
        distribution("let p: prob = 0.33\nreport typeof p")[0].0,
        Value::str("prob")
    );
    assert_eq!(type_of("typeof 33%"), "str");
}

#[test]
fn distribution_inspection_does_not_lift_or_draw() {
    for (expr, expected) in [
        ("d1", "dist[int]"),
        ("3d8", "dist[int]"),
        ("d6 > d6", "dist[bool]"),
        ("bernoulli(33%)", "dist[bool]"),
        ("normal(0, 1)", "dist[float]"),
        ("beta(2, 3)", "dist[float]"),
        ("binomial(5, 33%)", "dist[int]"),
        ("roll(2, d6)", "dist[list[int]]"),
        ("one_of([1, \"x\"])", "dist[any]"),
        ("one_of([normal(0, 1), 1.0])", "dist[float]"),
        ("one_of([normal(0, 1), 1])", "dist[any]"),
    ] {
        assert_eq!(type_of(expr), expected, "{expr}");
    }
    for mode in [Mode::Enumerate, Mode::Sample { runs: 100, seed: 7 }] {
        let out = exec(
            "let d = d6\nlet x ~ d\nreport [typeof d, typeof x]",
            &Options {
                mode: Some(mode),
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(
            out.reports[0].distribution(),
            vec![(Value::list(vec![Value::str("dist[int]"), Value::str("int")]), 1.0)]
        );
    }
}

#[test]
fn containers_named_types_and_empty_values() {
    for (expr, expected) in [
        ("[]", "list[unknown]"),
        ("[1, 2]", "list[int]"),
        ("[1, 2.0]", "list[any]"),
        ("[d6, d8]", "list[dist[int]]"),
        ("[[1], [2, 3]]", "list[list[int]]"),
        ("[[], [1]]", "list[any]"),
        ("[: ]", "map[unknown, unknown]"),
        ("[\"a\": 1, \"b\": 2]", "map[str, int]"),
        ("bag([])", "bag[unknown]"),
        ("bag([1: 2, 2: 1])", "bag[int]"),
        ("{ b: d6, a: 3 }", "{a: int, b: dist[int]}"),
    ] {
        assert_eq!(type_of(expr), expected, "{expr}");
    }
    let out = distribution(
        "enum S { A, B }\ntype Box = { value: int }\nreport [typeof A, typeof one_of([A, B]), typeof Box { value: 1 }]",
    );
    assert_eq!(
        out[0].0,
        Value::list(vec![Value::str("S"), Value::str("dist[S]"), Value::str("Box")])
    );
}

#[test]
fn operands_evaluate_once_and_keep_errors_and_world_effects() {
    let out = outcome("var n = 0\nlet t = typeof { n += 1; d6 }\nreport t\nreport n");
    assert_eq!(out.reports[0].distribution()[0].0, Value::str("dist[int]"));
    assert_eq!(out.reports[1].distribution()[0].0, Value::Int(1.into()));
    assert!(error("report typeof (1 / 0)").contains("zero"));
    assert!(compile_error("report typeof missing").contains("missing"));
    assert!(compile_error("let typeof = 1").contains("typeof"));
    let out = outcome("let t = typeof (if (chance { 50% => true, else => false }) { 1 } else { \"x\" })\nreport t");
    let dist = out.reports[0].distribution();
    assert_eq!(dist.len(), 2);
    for name in ["int", "str"] {
        close(dist.iter().find(|(v, _)| v == &Value::str(name)).unwrap().1, 0.5);
    }
    assert_eq!(
        type_of("simulate { if (chance { 50% => true, else => false }) { 1 } else { \"x\" } }"),
        "dist[any]"
    );
    assert_eq!(
        distribution("report [d6, d8].map(d -> typeof d)")[0].0,
        Value::list(vec![Value::str("dist[int]"), Value::str("dist[int]")])
    );
}

#[test]
fn inspection_preserves_sampling_and_conjugate_updates() {
    let base = "@mode sample(runs: 100, seed: 7)\nlet p ~ beta(1, 1)\n\
        observe 0 from binomial(100, prob(p))\nlet noise ~ normal(0, 1)\nreport p\nreport noise";
    let inspected = base.replace(
        "observe 0",
        "let kind = typeof p\nlet recipe = typeof normal(0, 1)\nobserve 0",
    );
    let before = outcome(base);
    let after = outcome(&inspected);
    assert_eq!(before.output, after.output);
    assert_eq!(before.stats.updates, after.stats.updates);
    let used = base
        .replace("observe 0", "let kind = typeof p\nobserve 0")
        .replace("report noise", "report noise\nreport kind");
    let out = outcome(&used);
    assert_eq!(out.reports[2].distribution()[0].0, Value::str("float"));
    // Computing with p still requires its value, unlike inspecting p itself.
    let forced = base.replace("observe 0", "let kind = typeof (p + 0)\nobserve 0");
    assert_ne!(outcome(&forced).stats.updates, before.stats.updates);
}

#[test]
fn inspection_retains_unresolved_mass_and_respects_limits() {
    let out = outcome(
        "@epsilon 0.1\nlet d = simulate { var n = 0; while (chance { 50% => true, else => false }) { n += 1 }; n }\nlet kind = typeof d\nreport d\nreport kind",
    );
    assert!(out.output.contains("unresolved"));
    assert_eq!(out.reports[1].distribution()[0].0, Value::str("dist[int]"));
    let e = exec(
        "report typeof d6",
        &Options {
            limits: Limits {
                max_string_bytes: 4,
                ..Limits::default()
            },
            ..Options::default()
        },
    )
    .unwrap_err();
    assert!(e.contains("string size"), "{e}");
    let e = error("var x = 0\nrepeat 70 { x = [x] }\nreport typeof x");
    assert!(e.contains("typeof value nesting"), "{e}");
}
