//! Cross-representation contracts agreed after the API consistency audit.
mod common;

use common::exec_raw;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Options};

fn values(mode: &str, source: &str) -> Vec<Value> {
    let out =
        exec_raw(&format!("@mode {mode}\n{source}"), &Options::default()).unwrap_or_else(|e| panic!("{source}\n{e:?}"));
    out.reports
        .iter()
        .map(|r| {
            let d = r.distribution();
            assert_eq!(d.len(), 1, "{source}: {d:?}");
            d[0].0.clone()
        })
        .collect()
}

fn facts(mode: &str, source: &str) {
    let vs = values(mode, source);
    assert!(!vs.is_empty());
    assert!(vs.iter().all(|v| *v == Value::Bool(true)), "{source}: {vs:?}");
}

fn rejects(mode: &str, source: &str, message: &str) {
    let e = exec_raw(&format!("@mode {mode}\n{source}"), &Options::default()).expect_err(source);
    assert_eq!(e.kind, ErrorKind::Language, "{source}: {e:?}");
    assert!(e.message.contains(message), "{source}: {e:?}");
}

fn relative(actual: f64, expected: f64) {
    assert!(actual.is_finite(), "{actual} versus {expected}");
    if expected == 0.0 {
        assert_eq!(actual, 0.0);
    } else {
        assert!((actual / expected - 1.0).abs() < 2e-12, "{actual} versus {expected}");
    }
}

macro_rules! both_modes {
    ($($test:item)*) => {
        mod enumerate { use super::*; const MODE: &str = "enumerate"; $($test)* }
        mod sample { use super::*; const MODE: &str = "sample(runs: 20, seed: 7)"; $($test)* }
    };
}

both_modes! {
    #[test]
    fn incomplete_queries_reject_all_representations() {
        let setup = "@epsilon 0.1\nlet d=simulate { var n=0; while 50% {n+=1}; n }";
        for population in ["d", "one_of([d,100])", "one_of([d,uniform(100,101)])"] {
            for query in [
                format!("cdf({population},200)"),
                format!("pmf({population},0)"),
                format!("P({population}<=200)"),
            ] {
                rejects(MODE, &format!("{setup}\nreport {query}"), "unresolved");
            }
        }
        rejects(
            MODE,
            "@epsilon 0.1\nlet mixed=simulate {var n=0; while 50% {n+=1}; uniform(n,n+1)}; report pdf(mixed,0.5)",
            "unresolved",
        );
        // Descriptive moments and support still describe retained outcomes.
        facts(
            MODE,
            &format!("{setup}\nreport mean(d)>=0 and median(d)>=0 and len(support(d))>0"),
        );
    }

    #[test]
    fn tiny_missing_mass_is_not_erased_by_lifting() {
        for query in [
            "cdf(geometric(50%),2)",
            "pmf(geometric(50%),1)",
            "P(geometric(50%)>1)",
            "P(abs(geometric(50%))>1)",
        ] {
            rejects(MODE, &format!("report {query}"), "unresolved");
        }
    }

    #[test]
    fn typed_keys_are_consistent_across_operations() {
        let keys = ["1", "1.0", "prob(1)", "complex(1)", "[1]", "[1.0]", "{x:1}", "{x:1.0}"];
        for (i, a) in keys.iter().enumerate() {
            for (j, b) in keys.iter().enumerate() {
                let same = i == j;
                facts(
                    MODE,
                    &format!(
                        r#"
                    let m=[{a}:"found"]; let bagged=bag([{a},{a}]); let k={b}
                    report m.contains(k)=={same}
                    report (m.get(k,"absent")=="found")=={same}
                    report (len(remove(m,k))==0)=={same}
                    report (len(insert(m,k,"new"))==1)=={same}
                    report bagged.contains(k)=={same} and (bagged.get(k)==2)=={same}
                    report if bagged.contains(k) {{ remove(bagged,k).get(k)==1 }} else {{ true }}
                "#
                    ),
                );
            }
        }
        rejects(MODE, "let m=[1:99]; report m[1.0]", "isn't in the map");
        rejects(MODE, "let b=bag([1]); report remove(b,1.0)", "isn't in the bag");
        facts(MODE, "let m=[1:10,1.0:20]; report m[1]==10 and m[1.0]==20");
        facts(
            MODE,
            "report 1==1.0 and [1].contains(1.0) and [1]!=[1.0] and {x:1}!={x:1.0}",
        );
    }

    #[test]
    fn typed_pmf_partitions_support_and_equality_queries_remain_explicit() {
        for population in ["xs", "one_of(xs)"] {
            facts(
                MODE,
                &format!(
                    r#"
                let xs=[1,1.0,prob(1),complex(1)]
                let d={population}
                report len(support(d))==4
                report support(d).map(x->pmf(d,x))==[prob(25%),prob(25%),prob(25%),prob(25%)]
                report sum(support(d).map(x->pmf(d,x)))==1
                report P(one_of(xs)==1)==100%
            "#
                ),
            );
        }
        facts(
            MODE,
            r#"
            let d=one_of([1,1.0,prob(1),complex(1),uniform(0,2)])
            report pmf(d,1)==20% and pmf(d,1.0)==20% and pmf(d,prob(1))==20% and pmf(d,complex(1))==20%
            report pmf(d,"1")==0% and pmf(uniform(0,2),1)==0%
            let huge=2^100
            let e=one_of([huge,huge+1,uniform(0,1)])
            report pmf(e,huge)==prob(1/3) and pmf(e,huge+1)==prob(1/3)
            let nested=one_of([[1],[1.0],{x:1},{x:1.0}])
            report pmf(nested,[1])==25% and pmf(nested,{x:1.0})==25%
        "#,
        );
    }

    #[test]
    fn computed_probabilities_remain_valid_at_boundaries() {
        for n in [3, 7, 10, 31, 100, 999] {
            let xs = (1..=n).map(|x| x.to_string()).collect::<Vec<_>>().join(",");
            for expr in [
                format!("cdf([{xs}],{n})"),
                format!("P(one_of([{xs}])<= {n})"),
                "cdf(one_of([uniform(0,1),uniform(1,2)]),2)".to_string(),
            ] {
                facts(
                    MODE,
                    &format!(
                        "let p={expr}; report p==100% and not p==0% and prob(p)==100%; report P(bernoulli(p))==100%; report if p {{true}} else {{false}}"
                    ),
                );
            }
        }
    }

    #[test]
    fn relative_weights_survive_scaling_and_duplicates() {
        for weights in [
            "[1:1,2:2,3:1]",
            "[1:5e307,2:1e308,3:5e307]",
            "[1:1e-300,2:2e-300,3:1e-300]",
        ] {
            facts(
                MODE,
                &format!(
                    "let d=one_of({weights}); report support(d)==[1,2,3] and pmf(d,1)==25% and pmf(d,2)==50% and pmf(d,3)==25%"
                ),
            );
        }
        facts(
            MODE,
            "let d=one_of([1:1e308, 2:1e308])*0; report support(d)==[0] and pmf(d,0)==100%",
        );
        let p = values(MODE, "report pmf(one_of([1:1e308,2:1e8]),2)")[0]
            .as_f64()
            .unwrap();
        relative(p, 1e-300);
        rejects(MODE, "report one_of([1:0,2:0])", "positive weight");
        rejects(MODE, "report one_of([1:prob(0.2),2:prob(0.3)])", "not 100%");
    }

    #[test]
    fn representable_moments_survive_large_locations_and_scales() {
        for (expr, expected) in [
            ("mean(uniform(-1e308,1e308))", 0.0),
            ("sd(uniform(-1e308,1e308))", 1e308 / libm::sqrt(3.0)),
            ("sd(one_of([-1e308,1e308]))", 1e308),
            ("sd([-1e308,1e308])", 1e308),
            ("sd(one_of([normal(-1e308,1),normal(1e308,1)]))", 1e308),
            ("mean(one_of([normal(1e308,1),normal(1.2e308,1)]))", 1.1e308),
            ("mean(triangular(1e308,1.2e308,1.4e308))", 1.2e308),
            ("mean(pert(1e308,1.2e308,1.4e308))", 1.2e308),
            ("variance(beta(1e308,1e308))", 1.25e-309),
            ("variance(gamma(1e308,1e-200))", 1e-92),
            ("sd(exponential(1e-308))", 1e308),
            ("quantile(uniform(-1e308,1e308),50%)", 0.0),
            ("pdf(uniform(-1e308,1e308),0)", 5e-309),
            ("cdf(normal(-1e308,1e308),1e308)", 0.9772498680518208),
            ("cdf(triangular(-1e308,0,1e308),0)", 0.5),
            ("pdf(triangular(-1e308,0,1e308),0)", 1e-308),
        ] {
            let v = values(MODE, &format!("report {expr}"));
            relative(v[0].as_f64().unwrap(), expected);
        }
        // CDF inversion at a symmetric median is accurate relative to the
        // distribution's scale, not to a zero-valued answer.
        facts(
            MODE,
            "let q=quantile(one_of([normal(-1e308,1e308),normal(1e308,1e308)]),50%); report abs(q/1e308)<1e-12",
        );
        for offset in [0_i64, 1_000_000_000, 1_000_000_000_000] {
            for family in ["triangular", "pert"] {
                let v = values(
                    MODE,
                    &format!("report variance({family}({offset},{},{}))", offset + 1, offset + 2),
                );
                relative(
                    v[0].as_f64().unwrap(),
                    if family == "triangular" { 1.0 / 6.0 } else { 1.0 / 7.0 },
                );
            }
        }
    }

    #[test]
    fn nonfinite_queries_fail_including_endpoints_and_singular_densities() {
        for expr in [
            "mean(lognormal(1000,1))",
            "sd(lognormal(1000,1))",
            "variance(normal(0,1e308))",
            "quantile(lognormal(1000,1),50%)",
            "quantile(normal(0,1),0%)",
            "quantile(normal(0,1),100%)",
            "pdf(beta(0.5,1),0)",
            "pdf(gamma(0.5,1),0)",
        ] {
            rejects(MODE, &format!("report {expr}"), "finite");
        }
        facts(
            MODE,
            "report quantile(uniform(0,1),0%)==0 and quantile(uniform(0,1),100%)==1 and pdf(uniform(0,0.1),0.05)==10",
        );
    }

    #[test]
    fn range_statistics_agree_with_uniform_integer_lists() {
        for lo in -3..=3 {
            for size in 1..=7 {
                let hi = lo + size - 1;
                let xs = (lo..=hi).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
                for query in ["mean", "variance", "sd", "median", "median_low", "median_high"] {
                    let vs = values(MODE, &format!("report {query}({lo}..{hi}); report {query}([{xs}])"));
                    relative(vs[0].as_f64().unwrap(), vs[1].as_f64().unwrap());
                }
                for q in ["0%", "10%", "25%", "30%", "50%", "75%", "100%", "0.30000000000000004"] {
                    facts(MODE, &format!("report quantile({lo}..{hi},{q})==quantile([{xs}],{q})"));
                }
                for x in [
                    format!("{lo}"),
                    format!("{hi}.0"),
                    format!("{}", hi + 1),
                    format!("{lo}-0.5"),
                ] {
                    facts(
                        MODE,
                        &format!("report cdf({lo}..{hi},{x})==cdf([{xs}],{x}) and pmf({lo}..{hi},{x})==pmf([{xs}],{x})"),
                    );
                }
            }
        }
    }

    #[test]
    fn large_ranges_keep_exact_ranks_and_do_not_materialize() {
        facts(
            MODE,
            r#"
            let n=2^2000
            report quantile(1..10,10%)==1 and quantile(1..10,30%)==3
            report median_low(0..n)==n div 2 and median_high(0..n)==n div 2
            report median(0..n)==n div 2
            report mean(-n..n)==0
            report quantile(1..n,50%)==n div 2
            report quantile(1..n,25%)==n div 4
            report quantile(1..n,0%)==1 and quantile(1..n,100%)==n
            report cdf(1..n,n div 2)==50%
            report quantile(1..2^54,0.5000000000000001)==2^53+2
            report quantile(1..2^1074,5e-324)==1
        "#,
        );
        for query in [
            "mean",
            "sd",
            "variance",
            "median",
            "median_low",
            "median_high",
            "support",
        ] {
            rejects(MODE, &format!("report {query}(1..0)"), "nonempty");
        }
        rejects(MODE, "report mean(0..2^2000)", "finite");
        rejects(MODE, "report variance(0..2^2000)", "finite");
    }

    #[test]
    fn sequence_lookup_and_extrema_share_validation() {
        facts(
            MODE,
            r#"
            let s="é🦀a"
            report s.get(0)==s[0] and s.get(1.0)==s[1] and s.get(3,"?")=="?" and s.get(-1,"?")=="?"
            report "".get(0,"?")=="?"
            report (2^100..2^100+2).get(1.0)==2^100+1 and (2..4).get(3,-1)==-1
            report minimum(s)=="a" and maximum(s)=="🦀" and minimum(["x"])=="x"
            report min(3,1,2)==1 and max(3,1,2)==3
            report P(min(d6,3)<=3)==100%
        "#,
        );
        for coll in ["\"\"", "\"abc\"", "1..3", "1..0"] {
            for key in ["0.5", "\"x\"", "true"] {
                rejects(MODE, &format!("report ({coll}).get({key},0)"), "index");
            }
        }
        for op in ["minimum", "maximum"] {
            rejects(MODE, &format!("report {op}(4)"), "expects");
            rejects(MODE, &format!("report {op}([complex(1)])"), "ordering");
            rejects(MODE, &format!("report {op}([{{x:1}}])"), "compare");
            rejects(MODE, &format!("report {op}(\"\")"), "empty");
        }
    }
}

#[test]
fn strict_and_computed_probability_boundaries_are_distinct() {
    use probl_engine::ops::{make_prob, to_prob};
    for p in [f64::NAN, f64::INFINITY, -f64::EPSILON, 1.0 + f64::EPSILON] {
        assert!(make_prob(&Value::Prob(p)).is_err());
        assert!(to_prob(&Value::Prob(p)).is_err());
    }
}

#[test]
fn large_range_scalar_queries_work_under_small_collection_limits() {
    let options = Options {
        limits: probl_engine::Limits {
            max_collection: 4,
            ..probl_engine::Limits::default()
        },
        ..Options::default()
    };
    exec_raw("report median(1..10^100); report cdf(1..10^100,5*10^99)", &options).unwrap();
    let e = exec_raw("report support(1..10^100)", &options).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Limit);
}

#[test]
fn analytic_reports_preserve_large_sd_and_label_unrepresentable_summaries() {
    let out = exec_raw("let x ~ normal(0,1e308); report x", &Options::default()).unwrap();
    let population = out.reports[0].distribution();
    let m = probl_engine::report::analytic_mixture(&population).unwrap();
    relative(m.sd(), 1e308);
    let out = exec_raw("report lognormal(1000,1)", &Options::default()).unwrap();
    assert!(out.output.contains("out of range"), "{}", out.output);
    assert!(!out.output.contains("NaN"));
}
