//! Regression cases from the language and use-case reviews.
mod common;

use common::*;
use probl_engine::{Limits, Options};
use probl_sema::ir::{Mode, ReportKind};

#[test]
fn callbacks_reject_effects_in_both_modes_before_results_can_hide_them() {
    let cases = [
        "report [1, 2].map(x -> { let r ~ d6; x + r })",
        "report [1].filter(x -> { let r ~ d6; r > 3 })",
        "report [1].count(x -> { let r ~ d6; r > 3 })",
        "report [1].reduce(0, (a, x) -> { let r ~ d6; a + r })",
        "report [1].map(x -> { let r ~ d1; x })",
        "report [1].map(x -> { observe true; x })",
        "report [1].map(x -> if 50% { x } else { x })",
        "report [1].map(x -> chance { 50% => x, else => x })",
        "report [1].map(x -> { var deck = bag([1: 1]); let r ~ deck.take(); r })",
        "fn draw() { let r ~ d6; r }\nreport [1].map(x -> draw())",
        // A cached one-outcome helper must not bypass the effect restriction.
        "fn draw() { let r ~ d1; r }\nlet cached = draw()\nreport [cached].map(x -> draw())",
        "report [1].map(x -> [2].map(y -> { observe true; y }))",
    ];
    for src in cases {
        for mode in [Mode::Enumerate, Mode::Sample { runs: 10, seed: 19 }] {
            for memoize in [false, true] {
                let options = Options {
                    mode: Some(mode.clone()),
                    memoize,
                    ..Options::default()
                };
                let e = exec(src, &options).expect_err(src);
                assert!(
                    e.contains("can't branch on chances, draw values or observe"),
                    "{src}: {e}"
                );
            }
        }
    }
}

#[test]
fn deterministic_callbacks_and_local_simulations_remain_usable() {
    let src = "let result = [1, 2].map(x -> if x > 1 { x + 2 } else { x })\n\
               report result\n\
               let means = [1, 2].map(x -> mean(simulate { let r ~ d6; observe r > 3; r + x }))\n\
               report abs(means[0] - 6) < 1e-12 and abs(means[1] - 7) < 1e-12\n\
               let r ~ d6\nreport r > 0";
    for mode in [Mode::Enumerate, Mode::Sample { runs: 100, seed: 19 }] {
        let out = exec(
            src,
            &Options {
                mode: Some(mode),
                ..Options::default()
            },
        )
        .unwrap();
        assert!(out.output.contains("[1, 4]"), "{}", out.output);
        close(out.reports[1].chance().unwrap(), 1.0);
    }
}

#[test]
fn repeated_and_shadowed_keys_are_per_visit() {
    let cases = [
        "let doubled ~ bernoulli(50%)\nlet keys = if doubled { [1, 1] } else { [1] }\n\
         for key in keys { report doubled by key as \"doubled\" }",
        "for outer in 1..2 { for key in 1..2 { report outer by key } }",
        "for key in 1..3 { let key = 1; report true by key }",
        "for key in [1, 1] { report true by key }",
    ];
    for src in cases {
        let (p, _) = probl_sema::compile(src);
        assert_eq!(p.unwrap().reports[0].kind, ReportKind::PerVisit, "{src}");
        assert!(output(src).contains("(per visit)"), "{src}");
    }
    assert!(output(cases[0]).contains("66.67%")); // Count visits, without deduplicating.
    let (p, _) = probl_sema::compile("for key in 1..3 { report true by key }");
    assert_eq!(p.unwrap().reports[0].kind, ReportKind::PerKey);
}

#[test]
fn two_contributing_samples_do_not_look_certain() {
    let src = "@mode sample(runs: 1000, seed: 1)\nlet visited ~ bernoulli(0.1%)\n\
               if visited { let outcome ~ bernoulli(50%); report outcome as \"rare branch\" }";
    let text = output(src);
    assert!(text.contains("95% Wilson interval"), "{text}");
    assert!(text.contains("2 contributing runs"), "{text}");
    assert!(!text.contains("100.00% ± 0.00%"), "{text}");
}

#[test]
fn no_successes_and_single_run_reports_show_nonzero_uncertainty() {
    for src in [
        "@mode sample(runs: 1000, seed: 1)\nlet hit ~ bernoulli(1e-15)\nreport hit",
        "@mode sample(runs: 1, seed: 1)\nreport false by 1",
    ] {
        let text = output(src);
        assert!(text.contains("95% Wilson interval"), "{text}");
        assert!(!text.contains("± 0.00%"), "{text}");
    }
}

#[test]
fn sparse_numeric_and_categorical_tables_show_per_key_support() {
    let src = "@mode sample(runs: 1000, seed: 1)\nlet visited ~ bernoulli(0.1%)\n\
               if visited { report 1 by 0\nreport \"seen\" by 0 }";
    let out = outcome(src);
    assert_eq!(out.output.matches("2 contributing runs").count(), 2, "{}", out.output);
    for sink in out.reports {
        for acc in sink.groups.values() {
            assert_eq!(acc.contributing_runs(), 2);
            close(acc.effective(), 2.0);
        }
    }
}

#[test]
fn unequal_weights_warn_even_when_many_runs_contributed() {
    let src = "@mode sample(runs: 1000, seed: 1)\nlet rare ~ bernoulli(0.1%)\n\
               observe if rare { 100% } else { 1e-10 }\nreport rare\nreport rare by 0";
    let out = outcome(src);
    assert!(
        out.output
            .contains("1,000 contributing runs; effective sample size 2.0"),
        "{}",
        out.output
    );
    assert!(!out.output.contains("Wilson"), "{}", out.output);
    assert_eq!(out.reports[0].groups.values().next().unwrap().contributing_runs(), 1000);
}

#[test]
fn per_visit_reports_do_not_claim_binomial_intervals() {
    let text = output("@mode sample(runs: 10, seed: 1)\nrepeat 2 { report true by 0 }");
    assert!(!text.contains("Wilson"), "{text}");
    assert!(text.contains("10 contributing runs"), "{text}");
    assert!(!text.contains("20 contributing runs"), "{text}");
}

#[test]
fn weighted_boundary_and_integrated_estimates_are_distinguished() {
    let weighted = output(
        "@mode sample(runs: 1000, seed: 7)\nlet x ~ d6\n\
                           observe if x == 6 { 90% } else { 10% }\nreport true",
    );
    assert!(!weighted.contains("Wilson"), "{weighted}");
    assert!(weighted.contains("MC error not estimable"), "{weighted}");
    let same_weights = output("@mode sample(runs: 1000, seed: 7)\nobserve 10%\nreport true");
    assert!(!same_weights.contains("Wilson"), "{same_weights}");
    let integrated = output("@mode sample(runs: 1000, seed: 7)\nreport d6 > 3");
    assert!(!integrated.contains("Wilson"), "{integrated}");
    assert!(
        integrated.contains("zero empirical MC error; integrated outcomes"),
        "{integrated}"
    );
}

#[test]
fn tiny_nonzero_summaries_and_table_cells_preserve_their_scale() {
    let text = output(
        "report one_of([1e-10, 3e-10])\n\
                       report one_of([-3e-10, -1e-10])\n\
                       for key in 1..2 { report one_of([1e-10, 3e-10]) by key }",
    );
    assert!(
        text.contains("mean 2.00e-10") && text.contains("mean -2.00e-10"),
        "{text}"
    );
    assert!(text.contains("sd 1.00e-10"), "{text}");
    assert!(!text.contains("0.00"), "{text}");
    let posterior = output(
        "@mode sample(runs: 2000, seed: 17)\n\
                            let p ~ beta(1,1)\nobserve 0 from binomial(100000,p)\nreport p",
    );
    assert!(!posterior.contains("mean 0.00"), "{posterior}");
    assert!(posterior.contains("e-5"), "{posterior}");
}

#[test]
fn cancellation_noise_is_distinguished_from_small_signals() {
    let cancelled = output("report one_of([-1.0, 1.0])\nreport one_of([-1e-10, 1e-10])");
    assert_eq!(cancelled.matches("mean ≈0").count(), 2, "{cancelled}");
    assert!(cancelled.contains("sd 1.00e-10"), "{cancelled}");
    // A tiny mean can still be meaningful even with outcomes of order one.
    let signal = output("report one_of([-1.0: 0.5 - 5e-11, 1.0: 0.5 + 5e-11])");
    assert!(signal.contains("mean 1.00e-10"), "{signal}");
}

#[test]
fn reliability_metadata_survives_batches_and_threads() {
    let src = "@mode sample(runs: 2500, seed: 7)\nlet x ~ d6\n\
               observe if x == 6 { 100% } else { 0.00001% }\n\
               report x == 6 by x\nreport x";
    let run = |threads| {
        exec(
            src,
            &Options {
                limits: Limits {
                    max_threads: threads,
                    ..Limits::default()
                },
                ..Options::default()
            },
        )
        .unwrap()
        .output
    };
    assert_eq!(run(1), run(4));
}
