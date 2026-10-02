mod common;
use common::*;
use probl_engine::value::Value;

fn value(src: &str) -> Value {
    distribution(src)[0].0.clone()
}

#[test]
fn percentages_are_numeric_and_probability_construction_is_explicit() {
    for expr in ["33%", "0%", "100%", "150%", "-33%"] {
        assert_eq!(value(&format!("report typeof ({expr})")), Value::str("float"));
    }
    for expr in [
        "prob(33%)",
        "prob(0.33)",
        "prob(true)",
        "prob(false)",
        "prob(0)",
        "prob(1)",
        "prob(prob(50%))",
    ] {
        assert_eq!(value(&format!("report typeof ({expr})")), Value::str("prob"));
    }
    assert_eq!(value("report prob(true)"), Value::Prob(1.0));
    assert_eq!(value("report prob(false)"), Value::Prob(0.0));
    for expr in [
        "prob(150%)",
        "prob(-0.1)",
        "prob(2^100)",
        "prob(d6)",
        "prob([0.3])",
        "prob(complex(0.3))",
        "P(0.3)",
    ] {
        error(&format!("report {expr}"));
    }
    assert_eq!(value("let p = 33%\nreport p == 0.33"), Value::Bool(true));
    for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(probl_engine::ops::make_prob(&Value::Float(n)).is_err());
    }
}

#[test]
fn literals_convert_at_typed_boundaries() {
    for src in [
        "let p: prob = 33%\nreport typeof p",
        "var p: prob = 0\np = 0.33\nreport typeof p",
        "fn f(p: prob) { typeof p }\nreport f(33%)",
        "fn f() -> prob { 33% }\nreport typeof f()",
        "fn f() -> prob { return 33% }\nreport typeof f()",
        "type R = { p: prob }\nlet r = R { p: 33% }\nreport typeof r.p",
        "let ps: list[prob] = [0, 33%, 1]\nreport typeof ps[1]",
    ] {
        assert_eq!(value(src), Value::str("prob"), "{src}");
    }
    for src in [
        "let x = 0.33\nlet p: prob = x\nreport p",
        "let p: prob = 0.3 + 0.03\nreport p",
        "fn f(p: prob) { p }\nlet x = 0.33\nreport f(x)",
        "fn f() -> prob { let x = 0.33; x }\nreport f()",
        "let x = 0.33\nreport bernoulli(x)",
        "let xs = [0.33]\nlet ps: list[prob] = xs\nreport ps",
        "report bernoulli(euler_gamma)",
        "let p: prob = euler_gamma\nreport p",
        "type R = { p: prob }\nlet r = R { p: 33% }\nreport r with { p: euler_gamma }",
    ] {
        error(src);
    }
    compile_error("let p: prob = 150%");
    compile_error("report bernoulli(150%)");
    close(chance("report bernoulli(33%)"), 0.33);
    close(chance("let x = 0.33\nreport bernoulli(prob(x))"), 0.33);
}

#[test]
fn literal_context_survives_later_argument_effects() {
    for source in [
        "fn g() { 1 }\nfn f(xs: list[prob], n: int) { typeof xs[0] }\nreport f([33%], g())",
        "fn g() -> prob { 50% }\nlet xs: list[list[prob]] = [[33%], [g()]]\nreport typeof xs[0][0]",
        "fn g() -> prob { 50% }\nfn f() -> list[list[prob]] { [[33%], [g()]] }\nreport typeof f()[0][0]",
        "fn g() { 1 }\ntype R = { ps: list[prob], n: int }\nlet r = R { ps: [33%], n: g() }\nreport typeof r.ps[0]",
        "fn g() -> prob { 50% }\nvar xs: list[list[prob]] = []\nxs.push([33%, g()])\nreport typeof xs[0][0]",
        "fn g() -> prob { 50% }\nfn f(xs: list[list[prob]]) { typeof xs[0][0] }\nreport [[33%], [g()]].f()",
    ] {
        assert_eq!(value(source), Value::str("prob"), "{source}");
    }
    let (program, _) = probl_sema::compile(
        "fn g() { print(\"called\"); 1 }\nfn f(xs: list[prob], n: int) { xs[0] }\nreport f([33%], g())",
    );
    let mut calls = Vec::new();
    probl_engine::run(&program.unwrap(), &Default::default(), &mut |s| {
        calls.push(s.to_owned())
    })
    .unwrap();
    assert_eq!(calls, ["called"]);
}

#[test]
fn probability_arithmetic_and_widening() {
    for expr in ["p + p", "p - p", "p * p", "p / p", "1 - p", "-p"] {
        assert_eq!(
            value(&format!("let p = prob(0.3)\nreport typeof ({expr})")),
            Value::str("float")
        );
    }
    assert_eq!(value("let p: float = prob(0.3)\nreport typeof p"), Value::str("float"));
    assert_eq!(
        value("fn f(x: float) { typeof x }\nreport f(prob(0.3))"),
        Value::str("float")
    );
    for (source, expected) in [
        ("let xs: list[float] = [prob(0.3)]\nreport typeof xs", "list[float]"),
        (
            "let xs: map[float, float] = [prob(0.3): prob(0.5)]\nreport typeof xs",
            "map[float, float]",
        ),
        (
            "let xs: bag[float] = bag([prob(0.3): 1, 0.3: 2])\nreport typeof xs",
            "bag[float]",
        ),
        (
            "let xs: dist[float] = one_of([prob(0.3), 0.3])\nreport typeof xs",
            "dist[float]",
        ),
        ("type R = { p: float }\nreport typeof R { p: prob(0.3) }.p", "float"),
    ] {
        assert_eq!(value(source), Value::str(expected), "{source}");
    }
    assert_eq!(
        value("let xs: bag[float] = bag([prob(0.3): 1, 0.3: 2])\nreport len(xs)"),
        Value::Int(3.into())
    );
    for expr in ["0.3 and 0.5", "prob(0.3) and prob(0.5)", "not prob(0.3)", "true + 1"] {
        error(&format!("report {expr}"));
    }
}

#[test]
fn conditions_and_observations_require_facts() {
    for expr in ["33%", "prob(33%)", "prob(0)", "prob(1)", "d6 > 3", "1"] {
        for src in [
            format!("if {expr} {{ report true }}"),
            format!("while {expr} {{ break }}\nreport true"),
            format!("observe {expr}\nreport true"),
            format!("report match 1 {{ _ if {expr} => true, _ => false }}"),
        ] {
            error(&src);
        }
    }
    let out = outcome("let x ~ 3d8\nobserve x > 10\nreport x");
    assert!(
        out.reports[0]
            .distribution()
            .iter()
            .all(|(v, _)| v.as_f64().unwrap() > 10.0)
    );
    close(chance("let e ~ bernoulli(5%)\nobserve e\nreport e"), 1.0);
    close(chance("report chance { 33% => true, else => false }"), 0.33);
    error("let x = 0.33\nreport chance { x => true, else => false }");
}

#[test]
fn weights_have_explicit_absolute_and_relative_modes() {
    close(chance("report one_of([true: 33%, false: 33%])"), 0.5);
    close(chance("report one_of([true: prob(33%), false: prob(67%)])"), 0.33);
    error("report one_of([true: prob(33%), false: prob(33%)])");
    error("report one_of([true: prob(33%), false: 67%])");
}

#[test]
fn explicit_conversion_preserves_conjugate_inference() {
    let base = "@mode sample(runs: 100, seed: 7)\nlet p ~ beta(1, 1)\nobserve 0 from binomial(100, prob(p))\nreport p";
    let before = outcome(base);
    let after = outcome(&base.replace("observe 0", "let kind = typeof p\nobserve 0"));
    assert_eq!(before.output, after.output);
    assert_eq!(before.stats.updates, after.stats.updates);
    assert!(before.stats.updates.iter().any(|u| u.exact > 0));
    error(&base.replace("prob(p)", "p"));
    error("@mode sample(runs: 10)\nlet p ~ beta(1, 1)\nobserve bernoulli(prob(p))\nreport p");
}

#[test]
fn typed_updates_preserve_probability_values() {
    for source in [
        "type R = { p: prob }\nlet r = R { p: 33% }\nreport typeof (r with { p: 50% }).p",
        "type R = { p: prob }\nvar r = R { p: 33% }\nr.p = 50%\nreport typeof r.p",
        "type R = { ps: list[prob] }\nlet r = R { ps: [33%] }\nreport typeof (r with { ps: [50%] }).ps[0]",
        "var xs: list[prob] = [33%]\nxs[0] = 50%\nreport typeof xs[0]",
        "var xs: list[prob] = []\nxs.push(50%)\nreport typeof xs[0]",
        "var xs: list[list[prob]] = [[]]\nxs[0].push(50%)\nreport typeof xs[0][0]",
        "var xs: map[str, prob] = [:]\nxs.insert(\"p\", 50%)\nreport typeof xs[\"p\"]",
        "fn g() { 1 }\ntype R = { ps: list[prob], n: int }\nlet r: R = R { ps: [33%], n: 0 }\nreport typeof (r with { ps: [50%], n: g() }).ps[0]",
        "type R = { p: prob }\nlet rs = one_of([R { p: 33% }, R { p: 50% }])\nlet r ~ rs with { p: 75% }\nreport typeof r.p",
    ] {
        assert_eq!(value(source), Value::str("prob"), "{source}");
    }
    for source in [
        "type R = { p: prob }\nlet r = R { p: 33% }\nlet x = 0.5\nreport r with { p: x }",
        "type R = { p: prob }\nvar r = R { p: 33% }\nlet x = 0.5\nr.p = x\nreport r",
        "var xs: list[prob] = [33%]\nlet x = 0.5\nxs[0] = x\nreport xs",
        "var xs: list[prob] = []\nlet x = 0.5\nxs.push(x)\nreport xs",
        "var xs: list[list[prob]] = [[]]\nlet x = 0.5\nxs[0].push(x)\nreport xs",
        "var xs: map[str, prob] = [:]\nlet x = 0.5\nxs.insert(\"p\", x)\nreport xs",
        "type R = { p: prob }\nlet rs = one_of([R { p: 33% }, R { p: 50% }])\nlet x = 0.5\nreport rs with { p: x }",
        "type R = { p: prob }\ntype S = { p: float }\nlet rs = one_of([R { p: 33% }, S { p: 0.5 }])\nlet x = 0.5\nreport rs with { p: x }",
        "var p: prob = 33%\np += 0.01\nreport p",
    ] {
        error(source);
    }
}
