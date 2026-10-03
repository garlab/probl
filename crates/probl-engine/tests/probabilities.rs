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
fn numbers_convert_at_typed_boundaries() {
    for src in [
        "let p: prob = 33%\nreport typeof p",
        "var p: prob = 0\np = 0.33\nreport typeof p",
        "fn f(p: prob) { typeof p }\nreport f(33%)",
        "fn f() -> prob { 33% }\nreport typeof f()",
        "fn f() -> prob { return 33% }\nreport typeof f()",
        "fn f() -> prob { if true { 33% } else { 50% } }\nreport typeof f()",
        "fn f(p: prob) { typeof p }\nreport f(if 30% { 33% } else { 50% })",
        "let p: prob = { let x = true; if x { 33% } else { 50% } }\nreport typeof p",
        "let p: prob = chance { 30% => 33%, else => 50% }\nreport typeof p",
        "type R = { p: prob }\nlet r = R { p: 33% }\nreport typeof r.p",
        "let ps: list[prob] = [0, 33%, 1]\nreport typeof ps[1]",
    ] {
        assert_eq!(value(src), Value::str("prob"), "{src}");
    }
    for src in [
        "let x = 0.33\nlet p: prob = x\nreport typeof p",
        "let p: prob = 0.3 + 0.03\nreport typeof p",
        "fn f(p: prob) { typeof p }\nlet x = 0.33\nreport f(x)",
        "fn f() -> prob { let x = 0.33; x }\nreport typeof f()",
        "fn f() -> prob { let x = 0.33; return x }\nreport typeof f()",
        "let xs = [0.33]\nlet ps: list[prob] = xs\nreport typeof ps[0]",
        "let p: prob = euler_gamma\nreport typeof p",
        "type R = { p: prob }\nlet r = R { p: 33% }\nreport typeof (r with { p: euler_gamma }).p",
        "fn f(p: prob) { typeof p }\nlet x = 0.33\nreport x.f()",
        "fn f(p: prob) { typeof p }\nlet g = x -> f(x)\nlet x = 0.33\nreport g(x)",
    ] {
        assert_eq!(value(src), Value::str("prob"), "{src}");
    }
    close(chance("let x = 0.33\nreport bernoulli(x)"), 0.33);
    close(chance("report bernoulli(euler_gamma)"), 0.5772156649015329);
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
fn contextual_conversion_preserves_source_values_and_evaluation_order() {
    for mode in ["enumerate", "sample(runs: 100, seed: 7)"] {
        let prefix = format!("@mode {mode}\n");
        for source in [
            "let x = 9%; let p: prob = x; report typeof x == \"float\" and typeof p == \"prob\"",
            "let xs = [0.3]; let ps: list[prob] = xs; report typeof xs[0] == \"float\" and typeof ps[0] == \"prob\"",
            "var x = 0.3; fn f(p: prob, q: prob) { p == 0.3 and q == 0.8 }; report f(x, { x = 0.8; x })",
            "var calls = 0; let p: prob = { calls += 1; 0.3 }; report p == 0.3 and calls == 1",
            "var x = 0.3; type R = { p: prob, q: prob }; let r = R { p: x, q: { x = 0.8; x } }; report r.p == 0.3 and r.q == 0.8",
            "let n = 0; report if n { false } else { true }",
            "let n = 1; report if n { true } else { false }",
            "let p = 0.0; var n = 0; while p { n += 1 }; report n == 0",
        ] {
            close(chance(&(prefix.clone() + source)), 1.0);
        }
    }
    close(
        mean("let churn = 9%; let lost ~ binomial(100, churn); report lost"),
        9.0,
    );
    close(mean("let churn = 9%; report binomial(100, 1 - churn)"), 91.0);
    close(mean("let p = 0.3; var n = 0; while p { n += 1 }; report n"), 0.3 / 0.7);
    close(
        chance("let p = 0.3; report match 1 { _ if p => true, _ => false }"),
        0.3,
    );
    close(
        outcome("let p = 0.3; score p; report true").evidence.unwrap().to_f64(),
        0.3,
    );
    close(mean("let p = 0.5; report quantile(d6, p)"), 3.0);
    close(mean("let p = 0.5; report geometric(p)"), 2.0);
    close(mean("let p = 0.5; report odds(p)"), 1.0);
    close(mean("let p = 0.5; report logit(p)"), 0.0);
}

#[test]
fn nested_declared_probability_types_convert_numeric_values() {
    for source in [
        "let xs = [[0.3]]; let ps: list[list[prob]] = xs; report typeof ps[0][0]",
        "let xs = [0.3: 0.7]; let ps: map[prob, prob] = xs; report typeof ps[prob(0.3)]",
        "var xs: bag[prob] = bag([0.3: 1, prob(0.3): 2]); let p = xs.take(); report typeof p",
        "let xs: dist[prob] = one_of([0.3, 0.7]); let p = ~xs; report typeof p",
        "let r = { p: 0.3 }; let s: { p: prob } = r; report typeof s.p",
        "fn f() -> list[prob] { let xs = [0.3]; xs }; report typeof f()[0]",
        "let p: prob = ~one_of([0.3, 0.7]); report typeof p",
    ] {
        assert_eq!(value(source), Value::str("prob"), "{source}");
    }
    close(
        mean("let xs: bag[prob] = bag([0.3: 1, prob(0.3): 2]); report len(xs)"),
        3.0,
    );
}

#[test]
fn contextual_conversion_rejects_invalid_numbers_and_unrelated_types() {
    for mode in ["enumerate", "sample(runs: 10, seed: 7)"] {
        for number in ["-0.1", "1.1", "2^100"] {
            for body in [
                "let p: prob = x; report p",
                "fn f(p: prob) { p }; report f(x)",
                "fn f() -> prob { x }; report f()",
                "report binomial(10, x)",
                "report bernoulli(x)",
                "report if x { true } else { false }",
                "report match 1 { _ if x => true, _ => false }",
                "while x { break }; report true",
                "report chance { x => true, else => false }",
                "score x; report true",
                "let ps: list[prob] = [x]; report ps",
            ] {
                error(&format!("@mode {mode}\nlet x = {number}; {body}"));
            }
        }
        for source in [
            "let x = true; let p: prob = x; report p",
            "let x = true; report bernoulli(x)",
            "let x = true; score x; report true",
            "let x = one_of([0.3, 0.7]); let p: prob = x; report p",
            "let x = one_of([0.3, 0.7]); score x; report true",
            "let x = one_of([0, 1]); report if x { true } else { false }",
            "let x = complex(0.3); report bernoulli(x)",
            "let x = 0.3; observe x; report true",
            "let x = 0.3; observe ~x; report true",
            "let x = 0.3; report x and true",
        ] {
            error(&format!("@mode {mode}\n{source}"));
        }
    }
    for p in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(probl_engine::ops::to_prob(&Value::Float(p)).is_err());
        assert!(probl_engine::ops::condition(&Value::Float(p)).is_err());
    }
    assert_eq!(value("let x = 0.3; report typeof (~x)"), Value::str("float"));
}

#[test]
fn implicit_probability_parameters_keep_exact_updates() {
    for evidence in [
        "observe 1 from binomial(2, p)",
        "observe true from bernoulli(p)",
        "observe ~bernoulli(p)",
        "score p",
    ] {
        let prefix = "@mode sample(runs: 100, seed: 7)\nlet p ~ beta(1, 1)\n";
        let implicit = outcome(&format!("{prefix}{evidence}\nreport p"));
        let explicit = evidence
            .replace(", p)", ", prob(p))")
            .replace("bernoulli(p)", "bernoulli(prob(p))")
            .replace("score p", "score prob(p)");
        let explicit = outcome(&format!("{prefix}{explicit}\nreport p"));
        assert_eq!(implicit.output, explicit.output, "{evidence}");
        assert_eq!(implicit.stats.updates, explicit.stats.updates, "{evidence}");
        assert!(implicit.stats.updates.iter().any(|u| u.exact == 100), "{evidence}");
    }
    error("@mode sample(runs: 10)\nlet p ~ beta(1, 1)\nobserve ~p\nreport p");
    error("@mode sample(runs: 10)\nlet p ~ normal(2, 0.00001)\nscore p\nreport p");
    assert_eq!(
        value("@mode sample(runs: 10)\nlet p: prob ~ beta(1, 1)\nreport typeof p"),
        Value::str("prob")
    );
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
    for expr in ["0.3 and 0.5", "true + 1"] {
        error(&format!("report {expr}"));
    }
}

#[test]
fn observations_require_facts() {
    for expr in ["33%", "prob(33%)", "prob(0)", "prob(1)", "d6 > 3", "1"] {
        error(&format!("observe {expr}\nreport true"));
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
    close(chance("let x = 0.33\nreport chance { x => true, else => false }"), 0.33);
}

#[test]
fn branching_accepts_probabilities_and_boolean_recipes() {
    for condition in ["30%", "0.3", "prob(30%)", "bernoulli(30%)"] {
        close(
            chance(&format!("report if {condition} {{ true }} else {{ false }}")),
            0.3,
        );
        close(
            chance(&format!("report match 1 {{ _ if {condition} => true, _ => false }}")),
            0.3,
        );
    }
    close(chance("report if d6 > 4 { true } else { false }"), 1.0 / 3.0);
    close(
        chance(
            "let p: prob = 30%\nlet a = if p { true } else { false }\nlet b = if p { true } else { false }\nreport a and b",
        ),
        0.09,
    );
    for condition in ["d6", "\"yes\"", "complex(0.3)"] {
        error(&format!("if {condition} {{ report true }}"));
    }
    close(chance("let p = 30%\nreport if p { true } else { false }"), 0.3);
    compile_error("if 150% { report true }");
}

#[test]
fn prefix_draws_have_identity_and_run_each_time() {
    close(chance("let p: prob = 30%\nlet a = ~p\nlet b = ~p\nreport a and a"), 0.3);
    close(
        chance("let p: prob = 30%\nlet a = ~p\nlet b = ~p\nreport a and b"),
        0.09,
    );
    close(mean("var rolls = 0\nwhile ~d6 != 6 { rolls += 1 }\nreport rolls"), 5.0);
    close(mean("var rolls = 0\nwhile 30% { rolls += 1 }\nreport rolls"), 0.3 / 0.7);
    assert_eq!(value("report typeof (~d6)"), Value::str("int"));
    assert_eq!(value("report typeof (~(d6 > 3))"), Value::str("bool"));
    assert_eq!(
        value("var calls = 0\nlet x = ~{ calls += 1; d6 }\nreport calls"),
        Value::Int(1.into())
    );
    assert_eq!(
        value("var x = 1\nlet xs = [x, ~{ x = 2; d1 }]\nreport xs[0]"),
        Value::Int(1.into())
    );
    assert_eq!(value("report false and ~(1 / 0)"), Value::Bool(false));
    for directive in ["@mode enumerate", "@mode sample(runs: 200, seed: 7)"] {
        assert_eq!(
            output(&format!("{directive}\nlet x = ~d6\nreport x")),
            output(&format!("{directive}\nlet x ~ d6\nreport x"))
        );
    }
}

#[test]
fn boolean_recipes_compose_independently() {
    close(chance("let d = d6 > 4\nreport d and d"), 1.0 / 9.0);
    close(chance("let d = d6 > 4\nreport d or d"), 5.0 / 9.0);
    close(chance("let d = d6 > 4\nlet a = ~d\nreport a and a"), 1.0 / 3.0);
    close(mean("let p: prob = 30%\nreport p and p"), 0.09);
    close(mean("let p: prob = 30%\nreport p or p"), 0.51);
    close(mean("let p: prob = 30%\nreport not p"), 0.7);
    close(chance("report prob(30%) and (d6 > 3)"), 0.15);
    assert_eq!(value("report typeof (prob(30%) and prob(40%))"), Value::str("prob"));
    assert_eq!(value("report typeof ((d6 > 3) and (d6 > 3))"), Value::str("dist[bool]"));
}

#[test]
fn boolean_laws_preserve_unresolved_mass() {
    use probl_engine::{
        dist::{Budget, Dist},
        ops,
    };
    let d = Dist::from_pairs(vec![(Value::Bool(true), 0.6), (Value::Bool(false), 0.2)], 0.2).into_value();
    let law = ops::boolean_law(&d).unwrap();
    assert_eq!(law, d);
    let cond = ops::condition(&law).unwrap();
    close(cond.yes, 0.6);
    close(cond.no, 0.2);
    close(cond.missing, 0.2);
    let result = ops::logic(
        true,
        ops::truth(&d, "and").unwrap(),
        ops::truth(&d, "and").unwrap(),
        &mut Budget::unlimited(),
    )
    .unwrap();
    let cond = ops::condition(&result).unwrap();
    close(cond.yes, 0.36);
    close(cond.no, 0.28);
    close(cond.missing, 0.36);
}

#[test]
fn score_and_observed_draws_apply_likelihoods() {
    for evidence in [
        "score if sick { 95% } else { 8% }",
        "let p: prob = if sick { 95% } else { 8% }; score p",
        "let p: prob = if sick { 95% } else { 8% }; observe ~p",
        "observe ~bernoulli(if sick { 95% } else { 8% })",
    ] {
        let result = outcome(&format!("let sick ~ bernoulli(1%)\n{evidence}\nreport sick"));
        close(result.evidence.unwrap().to_f64(), 0.0887);
        close(result.reports[0].chance().unwrap(), 0.0095 / 0.0887);
    }
    close(mean("let p: prob = 30%\nscore p\nreport p"), 0.3);
    close(
        chance("let p: prob = 30%\nlet event = ~p\nobserve event\nreport event"),
        1.0,
    );
    close(
        outcome("score match 1 { 1 => 30%, _ => 50% }\nreport true")
            .evidence
            .unwrap()
            .to_f64(),
        0.3,
    );
    close(
        outcome("score chance { 50% => 20%, else => 40% }\nreport true")
            .evidence
            .unwrap()
            .to_f64(),
        0.3,
    );
    for source in [
        "score true",
        "score d6",
        "score simulate { prob(30%) }",
        "let p = 130%\nscore p",
        "observe ~d6",
    ] {
        error(&format!("{source}\nreport true"));
    }
    compile_error("score 150%");
    compile_error("report true\nscore 30%");
    error("score 0%\nreport true");
}

#[test]
fn anonymous_observations_keep_conjugacy_and_bag_effects() {
    for evidence in ["score prob(p)", "observe ~prob(p)", "observe ~bernoulli(prob(p))"] {
        let result = outcome(&format!(
            "@mode sample(runs: 100, seed: 7)\nlet p = ~beta(1, 1)\n{evidence}\nreport p"
        ));
        assert!(result.stats.updates.iter().any(|u| u.exact == 100), "{evidence}");
        close(result.evidence.unwrap().to_f64(), 0.5);
    }
    let result = outcome("var cards = bag([true: 1, false: 1])\nobserve ~cards.take()\nreport len(cards)");
    close(result.evidence.unwrap().to_f64(), 0.5);
    assert_eq!(result.reports[0].distribution(), vec![(Value::Int(1.into()), 1.0)]);
    for mode in ["enumerate", "sample(runs: 10, seed: 7)"] {
        for body in ["score 50%; x", "~d6", "if 30% { x } else { x }"] {
            error(&format!("@mode {mode}\nreport [1].map(x -> {{ {body} }})"));
        }
    }
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
    let implicit = outcome(&base.replace("prob(p)", "p"));
    assert_eq!(before.output, implicit.output);
    assert_eq!(before.stats.updates, implicit.stats.updates);
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
        outcome(source);
        error(
            &source
                .replace("let x = 0.5", "let x = 1.5")
                .replace("p += 0.01", "p += 1"),
        );
    }
}
