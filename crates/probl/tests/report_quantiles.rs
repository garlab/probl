use probl::{ErrorKind, Limits, Options, ReportCollection, ReportSummary, SummaryError};

fn options(threads: usize) -> Options {
    let mut limits = Limits::default();
    limits.max_threads = threads;
    Options::new().runs(5007).seed(42).limits(limits)
}
fn quantiles() -> ReportCollection {
    ReportCollection::Quantiles { capacity: 128 }
}

#[test]
fn quantiles_preserve_draws_depletion_moments_and_worker_replay() {
    let p = probl::compile(
        "sketch",
        r#"
let x ~ normal(0, 1)
report max(0.0, x) as "balance"
report x <= 0 as "depleted"
report x as "draws"
"#,
    )
    .unwrap();
    let exact = p.run(&options(1).report_summary(0, ReportSummary::standard())).unwrap();
    let moments = p
        .run(&options(1).report_collection(0, ReportCollection::Moments))
        .unwrap();
    let sketch = p.run(&options(1).report_collection(0, quantiles())).unwrap();
    let parallel = p.run(&options(4).report_collection(0, quantiles())).unwrap();
    assert_eq!(sketch.text(), parallel.text());
    assert_eq!(sketch.render(1..3), exact.render(1..3));
    assert!(sketch.text().contains("approximate quantiles; inverse CDF"));
    assert_eq!(sketch.stats().world_steps(), exact.stats().world_steps());
    let group = &sketch.reports()[0].groups()[0];
    assert!(group.distribution().is_none());
    let n = group.numeric().unwrap();
    let metadata = n.approximation().unwrap();
    assert_eq!(metadata.method(), "compactor_v1");
    assert_eq!(metadata.capacity, 128);
    assert_eq!(metadata.samples, 5007);
    let p_zero = exact.reports()[1].groups()[0].probability().unwrap().point().unwrap();
    assert_eq!(metadata.zeros, (p_zero * 5007.0).round() as u64);
    assert_eq!(n.approximate_quantile(p_zero).unwrap().point(), Some(0.0));
    assert_eq!(n.quantile(0.5).unwrap_err(), SummaryError::Unavailable);
    assert_eq!(
        n.approximate_quantile(f64::NAN).unwrap_err(),
        SummaryError::InvalidQuantile
    );
    let exact_n = exact.reports()[0].groups()[0].numeric().unwrap();
    assert_eq!(
        exact_n.approximate_quantile(0.5).unwrap_err(),
        SummaryError::Unavailable
    );
    for q in [0.0, 0.05, 0.25, 0.5, 0.75, 0.95, 1.0] {
        let value = n.approximate_quantile(q).unwrap().point().unwrap();
        let lo = exact_n.quantile((q - 0.04).max(0.0)).unwrap().point().unwrap();
        let hi = exact_n.quantile((q + 0.04).min(1.0)).unwrap().point().unwrap();
        assert!(lo <= value && value <= hi, "q={q}: {lo} <= {value} <= {hi}");
    }
    let m = moments.reports()[0].groups()[0].numeric().unwrap();
    for (a, b) in [
        (n.mean().unwrap(), m.mean().unwrap()),
        (n.sd().unwrap(), m.sd().unwrap()),
    ] {
        assert_eq!(a.point(), b.point());
        assert_eq!(format!("{:?}", a.sampling()), format!("{:?}", b.sampling()));
    }
    #[cfg(feature = "json")]
    {
        let json = |o: &probl::Outcome| {
            serde_json::from_str::<serde_json::Value>(&o.reports_json(&Default::default()).unwrap()).unwrap()
        };
        let (a, b, c) = (json(&exact), json(&sketch), json(&parallel));
        assert_eq!(b, c);
        assert_eq!(a["run"], b["run"]);
        assert_eq!(a["reports"][1], b["reports"][1]);
        let s = &b["reports"][0]["groups"][0]["summary"];
        assert_eq!(s["quantile_method"], "compactor_v1");
        assert_eq!(s["approximation"]["samples"], "5007");
        assert!(s["approximation"]["rank_error_bound"].is_null());
        assert_eq!(s["quantiles"][2]["convention"], "inverse_cdf");
    }
}

#[test]
fn weighted_repeated_and_nonscalar_values_fail_explicitly() {
    for source in [
        "let x ~ d6; score 50%; report x",
        "for k in 1..3 { report k by 0 }",
        "report d6",
        "report [1, 2]",
        "report true",
        "report 9007199254740993",
        "let x ~ d6; report if x == 1 { true } else { 1 }",
    ] {
        let p = probl::compile("unsupported", source).unwrap();
        p.run(&options(1)).unwrap();
        let error = p.run(&options(1).report_collection(0, quantiles())).err().unwrap();
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{source}: {error}");
        assert!(error.to_string().contains("quantile collection"));
    }
}

#[test]
fn configuration_limits_and_unreached_groups() {
    let p = probl::compile("config", "print(\"not run\"); report 1").unwrap();
    for o in [
        Options::new().report_collection(0, quantiles()),
        options(1).report_collection(5, quantiles()),
        options(1).report_collection(0, ReportCollection::Quantiles { capacity: 0 }),
        options(1).report_collection(0, ReportCollection::Quantiles { capacity: usize::MAX }),
    ] {
        let mut printed = false;
        assert!(p.run_with(&o, &mut |_| printed = true).is_err());
        assert!(!printed);
    }
    for (source, expected) in [("report 1", true), ("if false { report 1 }", false)] {
        let p = probl::compile("budget", source).unwrap();
        let mut limits = Limits::default();
        limits.max_report_summary_bytes = 0;
        assert_eq!(
            p.run(&options(1).limits(limits).report_collection(0, quantiles()))
                .is_err(),
            expected
        );
    }
    let p = probl::compile("groups", "let x ~ d6; report x by x").unwrap();
    let mut limits = Limits::default();
    limits.max_report_groups = 2;
    assert!(
        p.run(&options(1).limits(limits).report_collection(0, quantiles()))
            .err()
            .unwrap()
            .to_string()
            .contains("max_report_groups")
    );
}

#[test]
fn keys_partial_reach_and_exact_defaults_keep_their_semantics() {
    let p = probl::compile(
        "keys",
        r#"
let x ~ d6
if x > 2 { report x by { even: x mod 2 == 0, label: "group" } }
"#,
    )
    .unwrap();
    let a = p.run(&options(1)).unwrap();
    let b = p.run(&options(4).report_collection(0, quantiles())).unwrap();
    for (a, b) in a.reports()[0].groups().iter().zip(b.reports()[0].groups()) {
        assert_eq!(a.key(), b.key());
        let (a, b) = (a.numeric().unwrap(), b.numeric().unwrap());
        assert!((a.mean().unwrap().point().unwrap() - b.mean().unwrap().point().unwrap()).abs() < 1e-12);
        assert_eq!(
            a.mean().unwrap().sampling().unwrap().contributing_runs(),
            b.approximation().unwrap().samples
        );
    }
}

#[test]
fn tenfold_trials_fit_the_same_live_collector_allowance() {
    let p = probl::compile("bounded", "for year in 0..2 {let x~normal(0,1); report x by year}").unwrap();
    let mut limits = Limits::default();
    limits.max_threads = 1;
    limits.max_report_summary_bytes = 4 * 1024 * 1024;
    for runs in [10_000, 100_000] {
        let o = p
            .run(
                &Options::new()
                    .runs(runs)
                    .seed(42)
                    .limits(limits.clone())
                    .report_collection(0, quantiles()),
            )
            .unwrap();
        for group in o.reports()[0].groups() {
            assert_eq!(group.numeric().unwrap().approximation().unwrap().samples, runs);
        }
    }
}
