//! Statistical functions query explicit populations; reports aggregate worlds.
mod common;
use common::*;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Limits, Options};

fn scalar(src: &str) -> Value {
    let d = distribution(src);
    assert_eq!(d.len(), 1, "{d:?}");
    close(d[0].1, 1.0);
    d[0].0.clone()
}

#[test]
fn statistical_queries_reject_scalar_inputs_in_both_modes() {
    for mode in ["enumerate", "sample(runs: 20, seed: 7)"] {
        for expression in [
            "mean(pi)",
            "median(pi)",
            "median_low(pi)",
            "median_high(pi)",
            "variance(2)",
            "sd(2)",
            "quantile(2,50%)",
            "cdf(2,3)",
            "pmf(2,2)",
            "support(2)",
            "pdf(2,2)",
            "mean(complex(1))",
            "median(\"abc\")",
        ] {
            let e = exec_raw(&format!("@mode {mode}\nreport {expression}"), &Options::default()).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Language, "{expression}: {e:?}");
            assert!(
                e.message.contains("expects") && e.message.contains("distribution"),
                "{e:?}"
            );
            assert!(e.help.as_deref().unwrap().contains("report x"), "{e:?}");
        }
        let e = error(&format!(
            "@mode {mode}\nlet x=if 50% {{ e }} else {{ pi }}\nprint(x,mean(x),median(x))"
        ));
        assert!(e.contains("expects a distribution") && e.contains("report x"), "{e}");
        for query in ["mean(x)", "median(x)", "median_low(x)", "median_high(x)", "P(x>1)"] {
            let e = exec_raw(
                &format!("@mode {mode}\nlet x ~ uniform(0,2)\nreport {query}"),
                &Options::default(),
            )
            .unwrap_err();
            assert_eq!(e.kind, ErrorKind::Language, "{query}: {e:?}");
            assert!(e.message.contains("expects"), "{e:?}");
        }
    }
}

#[test]
fn probability_queries_require_a_boolean_distribution() {
    for mode in ["enumerate", "sample(runs: 20, seed: 7)"] {
        for input in ["true", "false", "prob(30%)", "30%", "[true,false]", "d6"] {
            let e = error(&format!("@mode {mode}\nreport P({input})"));
            assert!(e.contains("dist[bool]"), "{input}: {e}");
        }
        close(mean(&format!("@mode {mode}\nreport P(d6>4)")), 1.0 / 3.0);
        close(mean(&format!("@mode {mode}\nreport P(bernoulli(30%))")), 0.3);
        close(mean(&format!("@mode {mode}\nreport prob(true)")), 1.0);
    }
}

#[test]
fn singletons_must_be_explicit_and_remain_valid() {
    for src in [
        "report mean([pi])",
        "report mean(one_of([pi]))",
        "report mean(simulate {pi})",
        "report median([pi])",
        "report median(one_of([pi]))",
    ] {
        close(scalar(src).as_f64().unwrap(), std::f64::consts::PI);
    }
    assert_eq!(scalar("report median([42])"), Value::Int(42.into()));
    close(mean("report variance([42])"), 0.0);
    close(mean("report P(one_of([true]))"), 1.0);
}

#[test]
fn lists_are_equal_weight_populations_with_duplicates() {
    for input in ["[1,1,4]", "one_of([1,1,4])"] {
        close(mean(&format!("report mean({input})")), 2.0);
        close(mean(&format!("report variance({input})")), 2.0);
        close(mean(&format!("report sd({input})")), 2.0f64.sqrt());
        close(mean(&format!("report cdf({input},1)")), 2.0 / 3.0);
        close(mean(&format!("report pmf({input},1)")), 2.0 / 3.0);
        assert_eq!(
            scalar(&format!("report support({input})")),
            Value::list(vec![Value::Int(1.into()), Value::Int(4.into())])
        );
    }
    close(mean("report mean([1,0.5,prob(75%)])"), 0.75);
    assert_eq!(scalar("report typeof mean([1,2,3])"), Value::str("float"));
    assert_eq!(scalar("report mean([1,2,3]) == [1,2,3].mean()"), Value::Bool(true));
}

#[test]
fn the_users_string_date_and_bigint_medians_select_an_element() {
    let src = r#"
let strings = ["aa", "cc", "bb"]
let dd = [today, today.add_months(2), today.add_workdays(3)]
let nn = [2^2^5, 2^3^5, 3^3^5]
report median_low(strings)
report median(dd)
report median(nn) == 2^3^5
report typeof median(nn)
report strings == ["aa", "cc", "bb"]
"#;
    let o = exec(
        src,
        &Options {
            today: probl_engine::dates::parse("2026-10-03"),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(o.reports[0].distribution(), vec![(Value::str("bb"), 1.0)]);
    assert_eq!(
        o.reports[1].distribution(),
        vec![(Value::Date(probl_engine::dates::parse("2026-10-07").unwrap()), 1.0)]
    );
    close(o.reports[2].chance().unwrap(), 1.0);
    assert_eq!(o.reports[3].distribution(), vec![(Value::str("int"), 1.0)]);
    close(o.reports[4].chance().unwrap(), 1.0);
    close(chance("let n=2^10000\nreport median([n+2,n,n+1]) == n+1"), 1.0);
    close(
        chance("report median([9007199254740993,9007199254740992.0,9007199254740994])==9007199254740993"),
        1.0,
    );
}

#[test]
fn list_medians_average_numbers_but_quantiles_select_elements() {
    close(scalar("report median([1,2,3,4])").as_f64().unwrap(), 2.5);
    assert_eq!(scalar("report median([1,3])"), Value::Int(2.into()));
    close(mean("report median([-2,-1])"), -1.5);
    close(mean("report median([1,2.0,3,4])"), 2.5);
    close(mean("report median([prob(25%),prob(75%)])"), 0.5);
    close(mean("report median([1,1,1,5])"), 1.0);
    close(mean("report median([1.7e308,1.7e308])"), 1.7e308);
    close(mean("report median([-1.7e308,1.7e308])"), 0.0);
    close(chance("report median([5e-324,5e-324]) == 5e-324"), 1.0);
    close(chance("let n=2^10000\nreport median([n,n+2]) == n+1"), 1.0);
    close(
        chance("report median([9007199254740992,9007199254740994]) == 9007199254740993"),
        1.0,
    );
    for input in ["[1,2,3,4]", "one_of([1,2,3,4])"] {
        assert_eq!(scalar(&format!("report quantile({input},50%)")), Value::Int(2.into()));
        assert_eq!(scalar(&format!("report quantile({input},0%)")), Value::Int(1.into()));
        assert_eq!(scalar(&format!("report quantile({input},100%)")), Value::Int(4.into()));
    }
    close(mean("report median(one_of([1,2,3,4]))"), 2.5);
    assert_eq!(scalar("report median_low([\"bb\",\"aa\"])"), Value::str("aa"));
    assert_eq!(scalar("report median_low([false,true])"), Value::Bool(false));
    close(mean("report cdf([false,true],false)"), 0.5);
    close(mean("report cdf(bernoulli(30%),false)"), 0.7);
    close(
        chance("report median([date(2026,3,1),date(2026,1,1)])==date(2026,1,30)"),
        1.0,
    );
    close(mean("report cdf([\"cc\",\"aa\",\"bb\"],\"bb\")"), 2.0 / 3.0);
    close(mean("report cdf([date(2026,1,1),date(2026,3,1)],date(2026,2,1))"), 0.5);
}

#[test]
fn complex_lists_have_means_but_no_order_or_real_variance() {
    let source = "let zz=[complex(1,2),complex(2,3),complex(0,1)]\nreport mean(zz)";
    let Value::Complex(z) = scalar(source) else {
        panic!("expected complex mean")
    };
    close(z.re(), 1.0);
    close(z.im(), 2.0);
    for expr in [
        "median([complex(1,2)])",
        "median([complex(1,2),complex(2,3),complex(0,1)])",
        "quantile([complex(1),complex(2)],50%)",
        "cdf([complex(1)],complex(1))",
    ] {
        assert!(error(&format!("report {expr}")).contains("no ordering"), "{expr}");
    }
    for name in ["sd", "variance"] {
        assert!(error(&format!("report {name}([complex(1),complex(2)])")).contains("real numeric elements"));
    }
}

#[test]
fn invalid_populations_fail_instead_of_becoming_point_masses() {
    for expr in [
        "mean([])",
        "median([])",
        "median_low([])",
        "median_high([])",
        "variance([])",
        "sd([])",
        "quantile([],50%)",
        "support([])",
        "cdf([],1)",
        "pmf([],1)",
    ] {
        assert!(error(&format!("report {expr}")).contains("nonempty list"), "{expr}");
    }
    for value in ["date(2026,1,1)", "\"aa\"", "true"] {
        for name in ["mean", "sd", "variance"] {
            if name == "mean" && value.starts_with("date") {
                continue;
            }
            let e = error(&format!("report {name}([{value}])"));
            assert!(e.contains("real numeric elements"), "{e}");
        }
    }
    assert!(error("report mean([2^10000])").contains("finite floats"));
    assert!(error("let n=2^10000\nreport median([n,n+1])").contains("fractional midpoint is too large"));
    assert!(error("report median([\"aa\",1])").contains("needs real numeric or date elements"));
    assert!(error("report median([d6,d8])").contains("list elements that are values"));
    assert!(error("let q=150%\nreport quantile([1,2],q)").contains("between 0 and 1"));
}

#[test]
fn lists_and_simulate_preserve_the_boundary_between_worlds() {
    let o = outcome("let xs=if 50% {[1,3]} else {[10,20]}\nreport mean(xs)");
    assert_eq!(
        o.reports[0].distribution(),
        vec![(Value::Float(2.0), 0.5), (Value::Float(15.0), 0.5)]
    );
    close(
        mean("report mean(simulate {if 50% {e} else {pi}})"),
        (std::f64::consts::E + std::f64::consts::PI) / 2.0,
    );
    // Capturing an already bound value doesn't rerun the outer model.
    let o = outcome("let x=if 50% {1} else {3}\nreport mean(simulate {x})");
    assert_eq!(
        o.reports[0].distribution(),
        vec![(Value::Float(1.0), 0.5), (Value::Float(3.0), 0.5)]
    );
}

#[test]
fn statistics_respect_host_work_limits() {
    let options = Options {
        limits: Limits {
            max_work: 500,
            ..Limits::default()
        },
        ..Options::default()
    };
    let xs = (1..=100).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
    exec_raw(&format!("report [{xs}]"), &options).unwrap();
    for name in ["median", "median_low", "median_high"] {
        let e = exec_raw(&format!("report {name}([{xs}])"), &options).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Limit);
    }
}

#[test]
fn median_variants_agree_for_lists_and_weighted_distributions_in_both_modes() {
    for mode in ["enumerate", "sample(runs: 20, seed: 7)"] {
        for (values, lo, mid, hi) in [
            ("[1,4]", 1.0, 2.5, 4.0),
            ("[4,1,1,4]", 1.0, 2.5, 4.0),
            ("[4,1,1]", 1.0, 1.0, 1.0),
            ("[4,1,2]", 2.0, 2.0, 2.0),
            ("[7]", 7.0, 7.0, 7.0),
        ] {
            for population in [values.to_owned(), format!("one_of({values})")] {
                for (name, expected) in [("median_low", lo), ("median", mid), ("median_high", hi)] {
                    close(mean(&format!("@mode {mode}\nreport {name}({population})")), expected);
                }
            }
        }
        for (weights, expected) in [
            ("[1:25%, 4:75%]", 4.0),
            ("[1:75%, 4:25%]", 1.0),
            ("[1:50.0000000001%, 4:49.9999999999%]", 1.0),
            ("[1:49.9999999999%, 4:50.0000000001%]", 4.0),
        ] {
            for name in ["median", "median_low", "median_high"] {
                close(
                    mean(&format!("@mode {mode}\nreport {name}(one_of({weights}))")),
                    expected,
                );
            }
        }
        for (name, expected) in [("median_low", 2.0), ("median", 3.0), ("median_high", 4.0)] {
            close(
                mean(&format!("@mode {mode}\nreport {name}(one_of([1:20%, 2:30%, 4:50%]))")),
                expected,
            );
        }
    }
    // Merging equal outcomes must not lose their multiplicity at the median.
    close(
        mean("let xs=(1..1000).map(x -> if x<=500 {1} else {4})\nreport median(xs)"),
        2.5,
    );
    close(
        mean("let xs=(1..1000).map(x -> if x<=500 {1} else {4})\nreport median(one_of(xs))"),
        2.5,
    );
    close(chance("let n=2^10000\nreport median(one_of([n,n+2]))==n+1"), 1.0);
    close(chance("report [1,4].median_low()==1 and [1,4].median_high()==4"), 1.0);
}

#[test]
fn ordinal_medians_must_be_explicit_even_for_singletons() {
    for values in ["[\"aa\"]", "[\"aa\",\"cc\",\"bb\"]", "[\"aa\",\"bb\"]", "[false,true]"] {
        for input in [values.to_owned(), format!("one_of({values})")] {
            let e = error(&format!("report median({input})"));
            assert!(e.contains("median_low") && e.contains("median_high"), "{e}");
        }
    }
    for input in ["[\"bb\",\"aa\"]", "one_of([\"bb\",\"aa\"])"] {
        assert_eq!(scalar(&format!("report median_low({input})")), Value::str("aa"));
        assert_eq!(scalar(&format!("report median_high({input})")), Value::str("bb"));
    }
    close(chance("report median_high(bernoulli(50%))"), 1.0);
    close(chance("report median_low(bernoulli(50%))"), 0.0);
    close(
        chance(
            "enum Size { Small, Large }\nreport median_low([Large,Small])==Small and median_high(one_of([Small,Large]))==Large",
        ),
        1.0,
    );
    assert!(error("enum Size { Small, Large }\nreport median([Small])").contains("median_low"));
    for name in ["median_low", "median_high"] {
        assert!(error(&format!("report {name}([complex(1)])")).contains("no ordering"));
        assert!(error(&format!("report {name}(one_of([complex(1)]))")).contains("no ordering"));
        assert!(error(&format!("report {name}([1,\"a\"])")).contains("can't compare"));
    }
}

#[test]
fn date_means_and_midpoint_medians_round_to_nearest_day_ties_earlier() {
    for mode in ["enumerate", "sample(runs: 20, seed: 7)"] {
        for (days, expected) in [("[1,9]", 5), ("[1,2]", 1), ("[1,1,2,2]", 1), ("[1,2,2]", 2), ("[2]", 2)] {
            // The rounding convention must be independent of the epoch.
            for year in [1, 1900, 1969, 1970, 2026, 9999] {
                for wrap in [false, true] {
                    let xs = format!("{days}.map(n -> date({year},1,n))");
                    let input = if wrap { format!("one_of({xs})") } else { xs };
                    for name in ["mean", "median"] {
                        close(
                            chance(&format!(
                                "@mode {mode}\nreport {name}({input})==date({year},1,{expected})"
                            )),
                            1.0,
                        );
                    }
                }
            }
        }
    }
    for name in ["mean", "median"] {
        close(
            chance(&format!(
                "report {name}([date(1969,12,31),date(1970,1,1)])==date(1969,12,31)"
            )),
            1.0,
        );
        assert!(error(&format!("report {name}([date(2026,1,1),1])")).contains("date"));
    }
    close(
        chance("report mean(one_of([date(2026,10,1):25%,date(2026,10,9):75%]))==date(2026,10,7)"),
        1.0,
    );
    close(
        chance("report median_low([date(2026,10,1),date(2026,10,9)])==date(2026,10,1)"),
        1.0,
    );
    close(
        chance("report median_high([date(2026,10,1),date(2026,10,9)])==date(2026,10,9)"),
        1.0,
    );
    close(
        chance("report mean([date(1,1,1),date(9999,12,31)])==median([date(1,1,1),date(9999,12,31)])"),
        1.0,
    );
}

#[test]
fn continuous_medians_respect_gaps_and_weights() {
    for (input, lo, mid, hi) in [
        ("normal(0,1)", 0.0, 0.0, 0.0),
        ("uniform(0,4)", 2.0, 2.0, 2.0),
        ("one_of([uniform(0,1),uniform(3,4)])", 1.0, 2.0, 3.0),
        ("one_of([-1,uniform(3,4)])", -1.0, 1.0, 3.0),
        (
            "one_of([uniform(0,1),uniform(3,4),uniform(3,4),uniform(3,4)])",
            10.0 / 3.0,
            10.0 / 3.0,
            10.0 / 3.0,
        ),
    ] {
        for (name, expected) in [("median_low", lo), ("median", mid), ("median_high", hi)] {
            close(mean(&format!("report {name}({input})")), expected);
        }
    }
}

#[test]
fn reports_use_midpoint_medians_too() {
    assert!(output("report one_of([1,4])").contains("median 2.50"));
    assert!(output("report one_of([date(2026,10,1),date(2026,10,9)])").contains("median 2026-10-05"));
    assert!(output("report one_of([uniform(0,1),uniform(3,4)])").contains("median 2.00"));
    assert!(output("let x ~ uniform(0,4)\nobserve x<=1 or x>=3\nreport x").contains("median 2.00"));
    let s = output("for i in 1..2 { report one_of([1,4]) by i }");
    assert!(s.contains("2.50"), "{s}");
}

#[test]
fn median_bounds_normalize_resolved_mass_and_budget_continuous_work() {
    use probl_engine::{
        builtins,
        continuous::Family,
        dist::{Budget, Dist},
    };
    use probl_sema::Builtin;
    use std::sync::Arc;
    let d = Dist::from_pairs(vec![(Value::Int(1.into()), 0.1), (Value::Int(4.into()), 0.1)], 0.8).into_value();
    for (b, expected) in [
        (Builtin::MedianLow, 1.0),
        (Builtin::Median, 2.5),
        (Builtin::MedianHigh, 4.0),
    ] {
        let v = builtins::call_raw(b, std::slice::from_ref(&d), &mut Budget::unlimited()).unwrap();
        close(v.as_f64().unwrap(), expected);
    }
    let d = Dist::uniform(
        (0..10)
            .map(|i| {
                Value::Continuous(Arc::new(Family::Uniform {
                    lo: i as f64,
                    hi: i as f64 + 1.0,
                }))
            })
            .collect(),
    )
    .into_value();
    for b in [Builtin::MedianLow, Builtin::Median, Builtin::MedianHigh] {
        let mut budget = Budget::unlimited();
        budget.work_left = 5;
        assert!(
            builtins::call_raw(b, std::slice::from_ref(&d), &mut budget)
                .unwrap_err()
                .message
                .contains("work budget")
        );
    }
}
