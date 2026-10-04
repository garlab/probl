//! Executable checks from docs/api-consistency-review.md (API-01 through API-09).
//!
//! All audit regressions are active. Design-choice checks reflect the agreed
//! typed-identity, explicit population and sequence contracts.

mod common;

use common::exec_raw;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Options, Outcome, RuntimeError};

fn run(mode: &str, source: &str) -> Result<Outcome, RuntimeError> {
    exec_raw(
        &format!("@mode {mode}\n{source}"),
        &Options {
            today: probl_engine::dates::parse("2026-10-04"),
            ..Options::default()
        },
    )
}

fn report_values(outcome: &Outcome) -> Vec<Value> {
    outcome
        .reports
        .iter()
        .map(|report| {
            let d = report.distribution();
            assert_eq!(d.len(), 1, "expected a deterministic result: {d:?}");
            assert_eq!(d[0].1, 1.0, "expected all report mass: {d:?}");
            d[0].0.clone()
        })
        .collect()
}

fn values(mode: &str, source: &str) -> Vec<Value> {
    let outcome = run(mode, source).unwrap_or_else(|e| panic!("{mode}: {source}\n{e:?}"));
    report_values(&outcome)
}

fn scalar(mode: &str, source: &str) -> Value {
    let mut vs = values(mode, source);
    assert_eq!(vs.len(), 1, "{source}");
    vs.pop().unwrap()
}

fn truth(mode: &str, source: &str) {
    assert_eq!(scalar(mode, source), Value::Bool(true), "{mode}: {source}");
}

fn number(mode: &str, source: &str) -> f64 {
    scalar(mode, source)
        .as_f64()
        .unwrap_or_else(|| panic!("expected a number: {source}"))
}

// No absolute 1e-9 floor: it would accept zero for a real 1e-13 tail.
// Dividing before subtracting also avoids overflow at large finite scales.
fn relative(actual: f64, expected: f64) {
    assert!(actual.is_finite(), "expected {expected}, got {actual}");
    if expected == 0.0 {
        assert_eq!(actual, 0.0);
    } else {
        assert!(
            (actual / expected - 1.0).abs() <= 1e-12,
            "expected {expected}, got {actual}"
        );
    }
}

fn language_error(mode: &str, source: &str, message: &str) {
    let e = run(mode, source).expect_err(source);
    assert_eq!(e.kind, ErrorKind::Language, "{source}: {e:?}");
    assert!(e.message.contains(message), "{source}: {e:?}");
}

const INCOMPLETE: &str = r#"
@epsilon 0.1
let d = simulate { var n = 0; while 50% { n += 1 }; n }
let a = one_of([d, 100])
let b = one_of([d, uniform(100, 101)])
"#;

// Both representations reject unresolved scalar probabilities under API-02.
fn incomplete_queries_agree(mode: &str, left: &str, right: &str) {
    for query in [left, right] {
        language_error(mode, &format!("{INCOMPLETE}\nreport {query}"), "unresolved");
    }
}

// Separate libtest cases ensure a failing enumeration assertion does not stop
// the matching sampling case from running.
macro_rules! both_modes {
    ($($test:item)*) => {
        mod enumerate {
            use super::*;
            const MODE: &str = "enumerate";
            $($test)*
        }
        mod sample {
            use super::*;
            const MODE: &str = "sample(runs: 20, seed: 7)";
            $($test)*
        }
    };
}

both_modes! {
    #[test]
    fn nested_sort_and_cdf_follow_numeric_lexicographic_order() {
        truth(
            MODE,
            r#"
    let xs = [[1.0, 0], [1, 100]]
    report sort(xs) == [[1.0, 0], [1, 100]] and cdf(xs, [1.0, 0]) == 50%
    "#,
        );
    }

    #[test]
    fn lower_median_agrees_with_language_order() {
        for input in ["xs", "one_of(xs)"] {
            for xs in ["[[1.0,0],[1,100]]", "[[1,100],[1.0,0]]"] {
                let actual = scalar(MODE, &format!("let xs={xs}\nreport median_low({input})"));
                assert_eq!(actual, Value::list(vec![Value::Float(1.0), Value::Int(0.into())]));
            }
        }
    }

    #[test]
    fn upper_median_agrees_with_language_order() {
        for input in ["xs", "one_of(xs)"] {
            let actual = scalar(MODE, &format!("let xs=[[1.0,0],[1,100]]\nreport median_high({input})"));
            assert_eq!(actual, Value::list(vec![Value::Int(1.into()), Value::Int(100.into())]));
        }
    }

    #[test]
    fn quantile_agrees_with_language_order() {
        for input in ["xs", "one_of(xs)"] {
            let actual = scalar(MODE, &format!("let xs=[[1.0,0],[1,100]]\nreport quantile({input},50%)"));
            assert_eq!(actual, Value::list(vec![Value::Float(1.0), Value::Int(0.into())]));
        }
    }

    #[test]
    fn reports_preserve_the_same_unresolved_bounds_for_both_mixtures() {
        let out = run(MODE, &format!("{INCOMPLETE}\nreport a <= 200\nreport b <= 200")).unwrap();
        for report in &out.reports {
            let acc = report.groups.get(&Value::Unit).unwrap();
            let (lo, hi) = acc.chance_bounds(out.unresolved);
            relative(lo, 31.0 / 32.0);
            assert_eq!(hi, 1.0);
        }
    }

    #[test]
    fn incomplete_cdf_has_one_normalization_contract() {
        incomplete_queries_agree(MODE, "cdf(a,200)", "cdf(b,200)");
    }

    #[test]
    fn incomplete_pmf_has_one_normalization_contract() {
        incomplete_queries_agree(MODE, "pmf(a,0)", "pmf(b,0)");
    }

    #[test]
    fn map_key_conversion_rejects_collisions() {
        for source in [
            "let original=[1: \"int\", 1.0: \"float\"]; let typed: map[prob,str]=original; report typed",
            "let original=[prob(0.5): \"prob\", 0.5: \"float\"]; let typed: map[float,str]=original; report typed",
            "let typed: map[prob,str]=[1: \"int\", 1.0: \"float\"]; report typed",
        ] {
            // The exact wording is not specified yet, but the failure must be
            // a language error, not a panic, a limit, or silent data loss.
            let e = run(MODE, source).expect_err("colliding keys must not lose a value");
            assert_eq!(e.kind, ErrorKind::Language, "{e:?}");
        }
    }

    #[test]
    fn bag_key_conversion_merges_counts_without_losing_items() {
        truth(
            MODE,
            r#"
    let original = bag([1, 1.0])
    let typed: bag[prob] = original
    report len(original) == 2 and len(typed) == 2 and typed.get(prob(1)) == 2
    "#,
        );
    }

    #[test]
    fn map_conversion_without_collisions_preserves_values_and_source() {
        truth(
            MODE,
            r#"
    let original = [0: "a", 1: "b"]
    let typed: map[prob,str] = original
    report (len(typed) == 2 and typed.get(prob(0)) == "a" and typed.get(prob(1)) == "b"
        and typeof original == "map[int, str]")
    "#,
        );
    }

    #[test]
    fn map_membership_predicts_removal() {
        for key in ["1.0", "prob(1)", "complex(1)"] {
            truth(
                MODE,
                &format!("let m=[1: \"a\"]; let k={key}; report contains(m,k) == (len(remove(m,k)) < len(m))"),
            );
        }
    }

    #[test]
    fn map_membership_predicts_whether_insert_replaces() {
        truth(
            MODE,
            r#"
    let m = [1: "a"]
    report contains(m,1.0) == (len(insert(m,1.0,"b")) == len(m))
    "#,
        );
    }

    #[test]
    fn bag_membership_agrees_with_count() {
        for key in ["1.0", "prob(1)", "complex(1)"] {
            truth(
                MODE,
                &format!("let b=bag([1]); let k={key}; report contains(b,k) == (get(b,k)>0)"),
            );
        }
    }

    #[test]
    fn a_bag_member_can_be_removed() {
        truth(
            MODE,
            "let b=bag([1]); report if contains(b,1.0) { len(remove(b,1.0)) == 0 } else { true }",
        );
    }

    #[test]
    fn pmfs_over_advertised_support_sum_to_one() {
        relative(
            number(MODE, "let d=one_of([1,1.0]); report sum(support(d).map(x -> pmf(d,x)))"),
            1.0,
        );
    }

    #[test]
    fn exact_typed_keys_work_for_map_and_bag_operations() {
        truth(
            MODE,
            r#"
    let m = [1: "a"]
    let b = bag([1,1])
    report (m.contains(1) and m.get(1) == "a" and len(remove(m,1)) == 0
        and insert(m,1,"b").get(1) == "b" and b.contains(1) and b.get(1) == 2
        and remove(b,1).get(1) == 1)
    "#,
        );
    }

    #[test]
    fn numeric_equality_events_are_distinct_from_typed_pmf() {
        // Numeric equality and list membership remain explicit value searches.
        truth(
            MODE,
            "report 1 == 1.0 and contains([1],1.0) and pmf([1,2],1.0) == 0% and P(one_of([1,1.0]) == 1) == 100%",
        );
    }

    #[test]
    fn map_lookup_agrees_with_membership_for_a_numeric_key_alias() {
        truth(
            MODE,
            "let m=[1: \"a\"]; report if contains(m,1.0) { get(m,1.0)==\"a\" } else { get(m,1.0,\"absent\")==\"absent\" }",
        );
    }

    #[test]
    fn container_equality_preserves_typed_contents() {
        // Container equality deliberately preserves typed contents.
        assert_eq!(
            values(
                MODE,
                "report [1]==[1.0]; report {x:1}=={x:1.0}; report contains([[1]],[1.0])"
            ),
            vec![Value::Bool(false); 3]
        );
    }

    #[test]
    fn computed_probabilities_and_complements_stay_in_range() {
        for n in 2..=32 {
            let items = (1..=n).map(|x| x.to_string()).collect::<Vec<_>>().join(",");
            for input in [format!("[{items}]"), format!("one_of([{items}])")] {
                for p in values(
                    MODE,
                    &format!("let p=cdf({input},{n}); report p; report not p; report prob(p)"),
                ) {
                    let Value::Prob(p) = p else {
                        panic!("expected prob, got {p:?}")
                    };
                    // No numerical tolerance is allowed for a type invariant.
                    assert!(p.is_finite() && (0.0..=1.0).contains(&p), "n={n}: {p}");
                }
            }
        }
    }

    #[test]
    fn explicit_out_of_range_probability_conversion_is_an_error() {
        for x in ["-0.0000000000000001", "1.0000000000000002"] {
            language_error(MODE, &format!("let x={x}; report prob(x)"), "between 0 and 1");
        }
    }

    #[test]
    fn continuous_mean_overflow_is_an_error() {
        language_error(MODE, "report mean(lognormal(1000,1))", "finite");
    }

    #[test]
    fn interior_continuous_quantile_overflow_is_an_error() {
        language_error(MODE, "report quantile(lognormal(1000,1),50%)", "finite");
    }

    #[test]
    fn continuous_variance_overflow_is_an_error() {
        language_error(MODE, "report variance(normal(0,1e308))", "finite");
    }

    #[test]
    fn finite_variance_and_continuous_median_already_reject_overflow() {
        language_error(MODE, "report variance(one_of([-1e308,1e308]))", "finite");
        language_error(MODE, "report median(lognormal(1000,1))", "finite");
    }

    #[test]
    fn normal_sd_preserves_a_large_representable_scale() {
        relative(number(MODE, "report sd(normal(0,1e308))"), 1e308);
    }

    #[test]
    fn uniform_mean_preserves_a_large_representable_midpoint() {
        relative(number(MODE, "report mean(uniform(1e308,1.2e308))"), 1.1e308);
    }

    #[test]
    fn symmetric_beta_mean_remains_one_half_at_large_scales() {
        relative(number(MODE, "report mean(beta(1e308,1e308))"), 0.5);
    }

    #[test]
    fn triangular_variance_is_translation_invariant() {
        for offset in [0_i64, 1_000, 1_000_000_000] {
            relative(
                number(
                    MODE,
                    &format!("report variance(triangular({offset},{},{}))", offset + 1, offset + 2),
                ),
                1.0 / 6.0,
            );
        }
    }

    #[test]
    fn ordinary_continuous_moments_match_independent_formulas() {
        for (expr, expected) in [
            ("mean(uniform(1,3))", 2.0),
            ("variance(uniform(1,3))", 1.0 / 3.0),
            ("sd(normal(0,2))", 2.0),
            ("mean(beta(2,2))", 0.5),
            ("variance(triangular(0,1,2))", 1.0 / 6.0),
        ] {
            relative(number(MODE, &format!("report {expr}")), expected);
        }
    }

    #[test]
    fn relative_weights_are_invariant_under_large_common_scaling() {
        let out = values(
            MODE,
            "let d=one_of([1:1e308,2:1e308]); report support(d); report pmf(d,1)",
        );
        assert_eq!(out[0], Value::list(vec![Value::Int(1.into()), Value::Int(2.into())]));
        relative(out[1].as_f64().unwrap(), 0.5);
    }

    #[test]
    fn ordinary_and_tiny_weights_preserve_their_ratios() {
        for weights in ["[1:1,2:1]", "[1:1e-300,2:1e-300]", "[1:3,2:3]"] {
            relative(number(MODE, &format!("report pmf(one_of({weights}),1)")), 0.5);
        }
        language_error(MODE, "report one_of([1:0,2:0])", "positive weight");
    }

    #[test]
    fn retained_tiny_tails_are_positive_and_have_the_right_mass() {
        let p = number(MODE, "let d=one_of([0:1,1000000:1e-13]); report pmf(d,1000000)");
        assert!(p > 0.0);
        relative(p, 1e-13 / (1.0 + 1e-13));
    }

    #[test]
    fn unordered_and_empty_list_quantiles_are_rejected() {
        for input in ["[1,\"a\"]", "[{x:1},{x:2}]"] {
            language_error(MODE, &format!("report quantile({input},50%)"), "compare");
        }
        language_error(MODE, "report quantile([],50%)", "nonempty");
    }

    #[test]
    fn heterogeneous_distribution_quantiles_are_rejected() {
        language_error(MODE, "report quantile(one_of([1,\"a\"]),50%)", "compare");
    }

    #[test]
    fn unordered_record_distribution_quantiles_are_rejected() {
        language_error(MODE, "report quantile(one_of([{x:1},{x:2}]),50%)", "compare");
    }

    #[test]
    fn hundredth_percentile_is_the_retained_finite_maximum() {
        assert_eq!(
            scalar(MODE, "let d=one_of([0:1,1000000:1e-13]); report quantile(d,100%)"),
            Value::Int(1_000_000.into())
        );
    }

    #[test]
    fn ordinary_quantile_endpoints_and_midpoint_medians_agree_across_populations() {
        for input in ["[1,4]", "one_of([1,4])"] {
            assert_eq!(
                values(
                    MODE,
                    &format!(
                        "report mean({input}); report median({input}); report median_low({input}); report median_high({input}); report quantile({input},0%); report quantile({input},100%)"
                    )
                ),
                vec![
                    Value::Float(2.5),
                    Value::Float(2.5),
                    Value::Int(1.into()),
                    Value::Int(4.into()),
                    Value::Int(1.into()),
                    Value::Int(4.into())
                ]
            );
        }
    }

    #[test]
    fn get_and_index_use_the_same_index_admissibility() {
        for expr in ["xs[1.0]", "xs.get(1.0,-1)"] {
            assert_eq!(
                scalar(MODE, &format!("let xs=[10,20]; report {expr}")),
                Value::Int(20.into())
            );
        }
    }

    #[test]
    fn get_does_not_hide_a_nonnumeric_index_behind_a_default() {
        language_error(MODE, "report get([10,20],\"wrong\",-1)", "index");
    }

    #[test]
    fn get_distinguishes_present_integer_indices_from_absent_positions() {
        assert_eq!(
            values(
                MODE,
                "let xs=[10,20]; report xs[1]; report xs.get(1,-1); report xs.get(2,-1)"
            ),
            vec![Value::Int(20.into()), Value::Int(20.into()), Value::Int((-1).into())]
        );
    }

    #[test]
    fn unicode_sequences_and_trim_use_scalar_characters() {
        assert_eq!(
            values(
                MODE,
                r#"
    let s = "é🦀a"
    report len(s)
    report s[1]
    report slice(s,1,2)
    report reverse(s)
    report sort(s)
    report "abbacacb".trim("ab")
    report s.chars().join("")
    "#
            ),
            vec![
                Value::Int(3.into()),
                Value::str("🦀"),
                Value::str("🦀"),
                Value::str("a🦀é"),
                Value::list(vec![Value::str("a"), Value::str("é"), Value::str("🦀")]),
                Value::str("cac"),
                Value::str("é🦀a")
            ]
        );
    }

    #[test]
    fn date_statistics_round_ties_earlier_and_transforms_preserve_calendar_rules() {
        let date = |s| Value::Date(probl_engine::dates::parse(s).unwrap());
        assert_eq!(
            values(
                MODE,
                r#"
    let ds = [date("2026-01-01"),date("2026-01-02")]
    report mean(ds)
    report median(ds)
    report median_low(ds)
    report median_high(ds)
    report date("2026-01-31").add_months(1)
    report date("2026-10-02").add_workdays(1)
    report today
    "#
            ),
            vec![
                date("2026-01-01"),
                date("2026-01-01"),
                date("2026-01-01"),
                date("2026-01-02"),
                date("2026-02-28"),
                date("2026-10-05"),
                date("2026-10-04")
            ]
        );
    }

    #[test]
    fn integer_and_complex_controls_keep_exactness_and_principal_values() {
        truth(
            MODE,
            r#"
    report (bit_length(0b1000) == 4 and ilog2(0xf) == 3
        and choose(60,30) == 118264581564861424 and gcd(84,30) == 6 and lcm(84,30) == 420
        and sum([complex(1,2),complex(2,3)]) == complex(3,5) and sum([]) == 0
        and ln(complex(-1)) == complex(0,pi) and erf(0) == 0)
    "#,
        );
    }

    #[test]
    fn enum_order_and_explicit_boolean_statistical_order_are_preserved() {
        truth(
            MODE,
            "enum S { z,a }; report z<a and sort([z,a])==[z,a] and median_low([z,a])==z",
        );
        assert_eq!(scalar(MODE, "report median_low([true,false])"), Value::Bool(false));
        relative(number(MODE, "report cdf([true,false],false)"), 0.5);
    }

    #[test]
    fn min_max_require_multiple_values_and_population_extrema_are_explicit() {
        for operation in ["min", "max"] {
            for scalar in ["1", "d6", "complex(1)"] {
                assert!(common::compile_error(&format!("report {operation}({scalar})")).contains("at least 2 arguments"));
            }
        }
        truth(
            MODE,
            "report minimum(d6)==1 and maximum(d6)==6 and P(min(d6,3)<=3)==100%",
        );
        assert_eq!(
            values(
                MODE,
                "report minimum(\"cba\"); report lowest(\"cba\",1)[0]; report minimum([\"c\",\"b\",\"a\"])"
            ),
            vec![Value::str("a"); 3]
        );
        language_error(MODE, "report highest(d6,1)", "needs a list");
    }

    #[test]
    fn sequence_capabilities_and_checked_slice_indices_are_explicit() {
        for query in ["mean(1..3)", "median(1..3)"] {
            relative(number(MODE, &format!("report {query}")), 2.0);
        }
        assert_eq!(
            scalar(MODE, "report slice([10,20],1.0)"),
            Value::list(vec![Value::Int(20.into())])
        );
        assert_eq!(scalar(MODE, "report \"abc\".get(1,\"?\")"), Value::str("b"));
        for query in ["sort([true,false])", "minimum([true,false])"] {
            language_error(MODE, &format!("report {query}"), "compare");
        }
    }

    #[test]
    fn complex_ordering_is_rejected_when_comparison_is_required() {
        for query in ["minimum([complex(1,2),complex(2,3)])", "sort([complex(1,2),complex(2,3)])"] {
            language_error(MODE, &format!("report {query}"), "no ordering");
        }
    }

    #[test]
    fn singleton_sort_and_min_validate_ordering() {
        language_error(MODE, "report minimum([complex(1,2)])", "no ordering");
        language_error(MODE, "report sort([complex(1,2)])", "no ordering");
    }

    #[test]
    fn round_digits_and_bag_defaults_have_documented_type_specific_behavior() {
        assert_eq!(
            values(
                MODE,
                "report typeof round(1.5); report typeof round(1.5,0); let b=bag([1]); report b.get(2); report b.get(2,-1)"
            ),
            vec![
                Value::str("int"),
                Value::str("float"),
                Value::Int(0.into()),
                Value::Int(0.into())
            ]
        );
    }

    #[test]
    fn pmf_absent_typed_outcomes_have_zero_mass() {
        relative(number(MODE, "report pmf([1,2],\"1\")"), 0.0);
        relative(number(MODE, "report pmf(uniform(1,2),\"1\")"), 0.0);
    }
}
