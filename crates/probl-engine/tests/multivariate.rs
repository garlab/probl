//! Language integration for sampling-only joint vector normals.
mod common;

use common::*;
use probl_engine::continuous::Rng;
use probl_engine::dist::Budget;
use probl_engine::multivariate::Normal;
use probl_engine::report::ReportCollection;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Limits, Options};
use probl_sema::ir::Mode;

fn sampled(runs: u64) -> Options {
    Options {
        mode: Some(Mode::Sample { runs, seed: 42 }),
        limits: Limits {
            max_threads: 1,
            ..Limits::default()
        },
        ..Options::default()
    }
}

#[test]
fn constructor_types_indirect_calls_and_stored_identity() {
    let source = r#"
fn make(m: list[float]): dist[list[float]] {
    let constructor = multivariate_normal
    constructor(m, [[1, 1], [1, 1]])
}
let annual: dist[list[float]] = make([0, 0])
let recipes = [annual, multivariate_normal([-0.0, 0.0], [[1.0, 1.0], [1.0, 1.0]])]
report typeof annual
report typeof one_of(recipes)
report len(keys(bag(recipes)))
report str(annual)
report map([0, 1], x -> typeof make([x, 0]))
"#;
    for options in [Options::default(), sampled(5)] {
        let out = exec_raw(source, &options).unwrap();
        assert_eq!(
            out.reports[0].distribution(),
            vec![(Value::str("dist[list[float]]"), 1.0)]
        );
        assert_eq!(out.reports[1].distribution(), out.reports[0].distribution());
        assert_eq!(out.reports[2].distribution(), vec![(Value::Int(1.into()), 1.0)]);
        assert_eq!(
            out.reports[3].distribution(),
            vec![(
                Value::str("multivariate_normal([0.0, 0.0], [[1.0, 1.0], [1.0, 1.0]])"),
                1.0
            )]
        );
    }
    assert!(error("let d: dist[float] = multivariate_normal([0], [[1]])\nreport typeof d").contains("dist[float]"));
}

#[test]
fn direct_draw_and_following_rng_match_the_kernel() {
    let source = r#"
let d = multivariate_normal([3, -2], [[4, 3], [3, 9]])
let x ~ d
let y ~ normal(0, 1)
report x
report y
report typeof x
"#;
    let out = exec_raw(source, &sampled(1)).unwrap();
    let mut budget = Budget::unlimited();
    let normal = Normal::new(&[3.0, -2.0], &[vec![4.0, 3.0], vec![3.0, 9.0]], &mut budget).unwrap();
    let mut rng = Rng::stream(42, 0);
    let expected = Value::list(
        normal
            .sample(&mut rng, &mut budget)
            .unwrap()
            .into_iter()
            .map(Value::Float)
            .collect(),
    );
    assert_eq!(out.reports[0].distribution(), vec![(expected, 1.0)]);
    assert_eq!(out.reports[1].distribution(), vec![(Value::Float(rng.normal()), 1.0)]);
    assert_eq!(out.reports[2].distribution(), vec![(Value::str("list[float]"), 1.0)]);
}

#[test]
fn singular_joint_draws_preserve_dependence_and_fresh_draws() {
    let source = r#"
let d = multivariate_normal([1, 2, 7], [[4, -6, 0], [-6, 9, 0], [0, 0, 0]])
let x ~ d
let y ~ d
report abs((x[0] - 1) * 3 + (x[1] - 2) * 2) < 1e-12
report x[2] == 7
report x != y
report x[0]
report x[1]
report (x[0] - 1) * (x[1] - 2)
"#;
    let out = exec_raw(source, &sampled(10_000)).unwrap();
    for i in 0..3 {
        close(out.reports[i].chance().unwrap(), 1.0);
    }
    for (i, expected) in [(3, 1.0), (4, 2.0), (5, -6.0)] {
        let group = out.reports[i].groups.values().next().unwrap();
        let (mean, se) = group.mean_se();
        assert!((mean - expected).abs() < 6.0 * se, "{i}: {mean} ± {se}");
    }
}

#[test]
fn finite_mixtures_draw_components_and_reports_draw_each_component() {
    let prefix = "let a = multivariate_normal([1], [[0]])\nlet b = multivariate_normal([2], [[0]])\nlet d: dist[list[float]] = one_of([a,b])\n";
    let out = exec_raw(
        &format!("{prefix}let x ~ d; let y ~ normal(0,1); report x; report y; report d"),
        &sampled(1),
    )
    .unwrap();
    let mut rng = Rng::stream(42, 0);
    let i = rng.choose([0.5, 0.5].into_iter()).unwrap();
    assert_eq!(
        out.reports[0].distribution(),
        vec![(Value::list(vec![Value::Float((i + 1) as f64)]), 1.0)]
    );
    assert_eq!(out.reports[1].distribution(), vec![(Value::Float(rng.normal()), 1.0)]);
    assert_eq!(
        out.reports[2].distribution(),
        vec![
            (Value::list(vec![Value::Float(1.0)]), 0.5),
            (Value::list(vec![Value::Float(2.0)]), 0.5)
        ]
    );

    let direct = exec_raw(
        "report multivariate_normal([0], [[1]]); let x ~ normal(0,1); report x",
        &sampled(1),
    )
    .unwrap();
    let mut rng = Rng::stream(42, 0);
    assert_eq!(
        direct.reports[0].distribution(),
        vec![(Value::list(vec![Value::Float(rng.normal())]), 1.0)]
    );
    assert_eq!(
        direct.reports[1].distribution(),
        vec![(Value::Float(rng.normal()), 1.0)]
    );
}

#[test]
fn recipes_returned_from_simulate_are_drawn_even_in_mixed_analytic_laws() {
    let source = "let d = simulate { let pick ~ d2; if pick == 1 { let x ~ normal(0,1); x } else { multivariate_normal([9], [[0]]) } }; let x ~ d; report typeof x";
    let out = exec_raw(source, &sampled(100)).unwrap();
    let distribution = out.reports[0].distribution();
    assert_eq!(distribution.len(), 2);
    assert!(distribution.iter().any(|(v, _)| *v == Value::str("float")));
    assert!(distribution.iter().any(|(v, _)| *v == Value::str("list[float]")));
    assert_eq!(
        exec_raw(source, &Options::default()).unwrap_err().kind,
        ErrorKind::Unsupported
    );
    let source = "let d = simulate { let pick ~ d2; multivariate_normal([pick], [[0]]) }; let x ~ d; report typeof x";
    assert_eq!(
        exec_raw(source, &sampled(100)).unwrap().reports[0].distribution(),
        vec![(Value::str("list[float]"), 1.0)]
    );
}

#[test]
fn cache_capacity_changing_parameters_and_workers_preserve_replay() {
    let source = r#"
fn make(m, variance) { multivariate_normal([m, 0], [[variance, 0.25], [0.25, 1]]) }
for year in 0..3 {
    let x ~ make(year, 1 + year mod 2)
    let y ~ make(-year, 1 + year mod 2)
    report x[0] + y[1] by year
}
let after ~ normal(0,1)
report after
"#;
    let expected = exec_raw(source, &sampled(4001)).unwrap().output;
    for (threads, entries) in [(1, 0), (1, 1), (4, 1), (7, 100)] {
        let mut options = sampled(4001);
        options.limits.max_threads = threads;
        options.limits.max_cached_calls = entries;
        assert_eq!(exec_raw(source, &options).unwrap().output, expected);
    }
}

#[test]
fn invalid_arguments_do_not_lift_and_domain_errors_can_be_caught() {
    for expression in [
        "multivariate_normal([d6], [[1]])",
        "multivariate_normal([0], [[d6]])",
        "multivariate_normal([true], [[1]])",
        "multivariate_normal([complex(0,1)], [[1]])",
        "multivariate_normal(0..1, [[1]])",
        "multivariate_normal([0], [1])",
        "multivariate_normal(one_of([[0],[1]]), [[1]])",
    ] {
        for options in [Options::default(), sampled(2)] {
            let e = exec_raw(&format!("report typeof {expression}"), &options).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Language, "{expression}: {e:?}");
        }
    }
    for expression in [
        "multivariate_normal([], [])",
        "multivariate_normal([0], [[1,0]])",
        "multivariate_normal([0,0], [[1,2],[2,1]])",
        "multivariate_normal([0,0], [[1,0],[0.1,1]])",
        "multivariate_normal([0,0], [[0,1],[1,1]])",
        "multivariate_normal([0], [[-1]])",
    ] {
        let src = format!("let ok = try {{ {expression}; false }} catch DomainError {{ true }}; report ok");
        close(exec_raw(&src, &sampled(2)).unwrap().reports[0].chance().unwrap(), 1.0);
    }
    let src = "let ok = try { multivariate_normal([2^2000], [[1]]); false } catch NumericOverflow { true }; report ok";
    close(chance(src), 1.0);
    assert!(
        error("let x ~ normal(0,1); report typeof multivariate_normal([x], [[1]])").contains("settled real number")
    );
}

#[test]
fn undrawn_vector_operations_are_explicitly_unsupported() {
    for recipe in [
        "multivariate_normal([0], [[1]])",
        "multivariate_normal([0], [[0]])",
        "one_of([multivariate_normal([0], [[1]]), multivariate_normal([1], [[1]])])",
    ] {
        for use_it in ["let x ~ d; report x", "report d", "report simulate { let x ~ d; x }"] {
            let e = exec_raw(&format!("let d = {recipe}; {use_it}"), &Options::default()).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Unsupported, "{use_it}: {e:?}");
        }
        for expression in [
            "mean(d)",
            "variance(d)",
            "sd(d)",
            "median(d)",
            "quantile(d,50%)",
            "minimum(d)",
            "maximum(d)",
            "support(d)",
            "P(d)",
            "pdf(d,[0])",
            "cdf(d,[0])",
            "pmf(d,[0])",
            "d[0]",
            "d + 1",
            "d == d",
            "-d",
            "map(d, x -> x)",
            "mixture([d])",
            "truncate(d,0,1)",
            "bins(d,10)",
        ] {
            let e = exec_raw(&format!("let d = {recipe}; report {expression}"), &sampled(2)).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Unsupported, "{expression}: {e:?}");
        }
        let e = exec_raw(
            &format!("let d = {recipe}; observe [0] from d; report true"),
            &sampled(2),
        )
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Unsupported);
        let e = exec_raw(
            &format!("let d = {recipe}; report simulate {{ let x ~ d; x }}"),
            &sampled(2),
        )
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Unsupported);
    }
}

#[test]
fn draws_in_callbacks_remain_forbidden_but_construction_is_pure() {
    let source = "fn draw() { let x ~ multivariate_normal([0], [[1]]); x }; report map([1], n -> draw())";
    for options in [Options::default(), sampled(1)] {
        let error = exec_raw(source, &options).unwrap_err();
        assert!(
            error
                .message
                .contains("can't branch on chances, draw values or observe"),
            "{error:?}"
        );
    }
    assert!(
        exec_raw(
            "report map([1], x -> typeof multivariate_normal([x], [[1]]))",
            &sampled(1)
        )
        .is_ok()
    );
}

#[test]
fn host_limits_cover_conversion_and_live_recipes_and_scalar_collectors() {
    let source = "let d = multivariate_normal([0,0], [[1,0],[0,1]]); let x ~ d; report x[0]";
    for limits in [
        Limits {
            max_multivariate_dimension: 1,
            ..Limits::default()
        },
        Limits {
            max_multivariate_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_work: 10,
            ..Limits::default()
        },
    ] {
        let mut options = sampled(1);
        options.limits = limits;
        assert_eq!(exec_raw(source, &options).unwrap_err().kind, ErrorKind::Limit);
    }
    for collection in [ReportCollection::Moments, ReportCollection::Quantiles { capacity: 128 }] {
        let mut options = sampled(2000);
        options.report_collections.insert(0, collection);
        assert!(exec_raw(source, &options).is_ok());
        assert!(exec_raw("report multivariate_normal([0], [[1]])", &options).is_err());
    }
}

#[test]
fn invalid_dynamic_covariance_preserves_partial_report_reach() {
    let source = "let v ~ one_of([-1,1]); let x ~ multivariate_normal([0], [[v]]); report x[0]";
    let out = exec_partial(source, &sampled(1000)).unwrap();
    assert!(!out.failures.is_empty());
    assert!(!out.reports[0].distribution().is_empty());
    assert!(out.failures.groups[0].error.message.contains("variance"));
}

#[test]
fn portfolio_example_agrees_with_independent_normal_formulas() {
    let out = exec_raw(
        include_str!("../../../examples/22_correlated_returns.probl"),
        &sampled(10_000),
    )
    .unwrap();
    let expected_mean: f64 = 0.6 * 0.05 + 0.4 * 0.02;
    let expected_variance: f64 = 0.6 * 0.6 * 0.04 + 0.4 * 0.4 * 0.01 + 2.0 * 0.6 * 0.4 * 0.006;
    let portfolio = out.reports[2].groups.values().next().unwrap();
    let (mean, se) = portfolio.mean_se();
    assert!((mean - expected_mean).abs() < 6.0 * se);
    let variance: f64 = out.reports[2]
        .distribution()
        .iter()
        .map(|(v, p)| (v.as_f64().unwrap() - mean).powi(2) * p)
        .sum();
    assert!((variance / expected_variance - 1.0).abs() < 0.05);
    let expected_loss = 0.5 * libm::erfc(expected_mean / libm::sqrt(2.0 * expected_variance));
    let loss = out.reports[3].groups.values().next().unwrap();
    assert!((loss.chance() - expected_loss).abs() < 6.0 * loss.chance_se());
}
