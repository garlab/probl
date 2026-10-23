use probl::{ErrorKind, Options, ReportStatistic as S, ReportSummary};

#[test]
fn selections_are_checked_and_preserve_column_order() {
    for text in [
        "",
        "mean,mean",
        "q:0.1,q:0.10",
        "q:NaN",
        "q:inf",
        "q:-0.1",
        "q:1.1",
        "median,",
        "unknown",
    ] {
        assert!(text.parse::<ReportSummary>().is_err(), "{text}");
    }
    assert!(ReportSummary::new(vec![]).is_err());
    assert!(
        format!("forecast{}", " ".repeat(4096))
            .parse::<ReportSummary>()
            .is_err()
    );
    assert!(ReportSummary::new((0..33).map(|i| S::Quantile(i as f64 / 33.0)).collect()).is_err());
    let selection: ReportSummary = "q:0.5,median,mean,sd,q:0,q:1".parse().unwrap();
    assert_eq!(
        selection.statistics(),
        &[
            S::Quantile(0.5),
            S::Median,
            S::Mean,
            S::StandardDeviation,
            S::Quantile(0.0),
            S::Quantile(1.0)
        ]
    );
    assert_eq!(S::Quantile(0.29).label(), "29%");
    assert_eq!(S::Quantile(0.001).label(), "0.1%");
}

#[test]
fn explicit_median_differs_from_the_inverse_cdf_at_half_mass() {
    let p = probl::compile(
        "columns",
        "report one_of([0:50%,10:50%]) as \"same\"; report d6 as \"same\"",
    )
    .unwrap();
    assert_eq!(p.report_labels().collect::<Vec<_>>(), ["same", "same"]);
    let options = Options::new().report_summary(0, "q:0.5,median,q:0,q:1,mean,sd".parse().unwrap());
    let outcome = p.run(&options).unwrap();
    assert!(
        outcome
            .text()
            .contains("50% 0 · median 5 · 0% 0 · 100% 10 · mean 5.00 · sd 5.00"),
        "{}",
        outcome.text()
    );
    assert_eq!(outcome.render(1..2), p.run(&Options::new()).unwrap().render(1..2));
    assert_eq!(
        outcome.render(0..1).trim(),
        outcome.text().lines().nth(2).unwrap().trim()
    );
}

#[test]
fn identity_errors_precede_model_side_effects() {
    let p = probl::compile("validation", "print(\"side effect\"); report d6").unwrap();
    let mut printed = Vec::new();
    let error = p
        .run_with(
            &Options::new().report_summary(1, ReportSummary::forecast()),
            &mut |line| printed.push(line.to_string()),
        )
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::Usage);
    assert!(printed.is_empty());
}

#[test]
fn dates_large_integers_and_incompatible_values_are_explicit() {
    let p = probl::compile(
        "typed",
        "report one_of([date(2026,1,1),date(2026,1,3)]); report one_of([9007199254740993,9007199254740995])",
    )
    .unwrap();
    let selection: ReportSummary = "median,q:0.5".parse().unwrap();
    let outcome = p
        .run(
            &Options::new()
                .report_summary(0, selection.clone())
                .report_summary(1, selection),
        )
        .unwrap();
    assert!(outcome.text().contains("median 2026-01-02 · 50% 2026-01-01"));
    assert!(
        outcome
            .text()
            .contains("median 9,007,199,254,740,994 · 50% 9,007,199,254,740,993")
    );
    assert!(
        p.run(&Options::new().report_summary(0, ReportSummary::moments()))
            .err()
            .unwrap()
            .to_string()
            .contains("date reports")
    );
    for source in [
        "report \"cat\"",
        "let x~normal(0,1); report {x:x}",
        "for k in 0..1 {report if k==0 {true} else {1} by k}",
    ] {
        assert!(
            probl::compile("types", source)
                .unwrap()
                .run(&Options::new().report_summary(0, ReportSummary::forecast()))
                .is_err(),
            "{source}"
        );
    }
    let p = probl::compile("facts", "report d6>3").unwrap();
    assert_eq!(
        p.run(&Options::new()).unwrap().text(),
        p.run(&Options::new().report_summary(0, ReportSummary::moments()))
            .unwrap()
            .text()
    );
}

#[test]
fn a_value_the_summary_cant_describe_stops_the_run_where_it_is_reported() {
    // The `print` after each report would run if the values were only
    // checked once the run had finished.
    for (source, summary, problem) in [
        ("report \"cat\"", "forecast", "require numeric or date values"),
        ("report [1, 2]", "median", "require numeric or date values"),
        ("report date(2026, 1, 1)", "mean", "date reports"),
        (
            "for k in 0..1 { report if k == 0 { true } else { 1 } by k }",
            "forecast",
            "mixed Boolean",
        ),
        (
            "@mode sample(runs: 100, seed: 7)\nlet x ~ d6\nreport if x == 1 { true } else { x }",
            "forecast",
            "mixed Boolean",
        ),
    ] {
        let p = probl::compile("early", &format!("{source}\nprint(\"after\")")).unwrap();
        let mut printed = Vec::new();
        let error = p
            .run_with(
                &Options::new().report_summary(0, summary.parse().unwrap()),
                &mut |line| printed.push(line.to_string()),
            )
            .err()
            .unwrap();
        assert!(error.to_string().contains(problem), "{source}: {error}");
        assert!(printed.is_empty(), "{source}: {printed:?}");
    }
    // A distribution is known by its outcomes, so it's still checked after
    // the run, with the same message.
    let p = probl::compile("late", "report one_of([\"a\", \"b\"])").unwrap();
    let error = p
        .run(&Options::new().report_summary(0, ReportSummary::forecast()))
        .err()
        .unwrap();
    assert!(error.to_string().contains("require numeric or date values"), "{error}");
}

#[cfg(feature = "json")]
#[test]
fn json_selection_and_text_agree_without_changing_sampled_results() {
    let p = probl::compile(
        "sample",
        "@mode sample(runs:100,seed:42)\nfor year in 0..2 {let x~normal(0,1); report x by year; report x>0 by year}",
    )
    .unwrap();
    let standard = p.run(&Options::new()).unwrap();
    let chosen = p
        .run(&Options::new().report_summary(0, ReportSummary::forecast()))
        .unwrap();
    let json = |out: &probl::Outcome| {
        serde_json::from_str::<serde_json::Value>(&out.reports_json(&probl::ReportJsonLimits::default()).unwrap())
            .unwrap()
    };
    let a = json(&standard);
    let b = json(&chosen);
    assert_eq!(a["reports"][1], b["reports"][1]);
    assert_eq!(a["run"], b["run"]);
    assert_eq!(
        b["reports"][0]["summary_selection"],
        serde_json::json!(["q:0.1", "q:0.25", "median", "q:0.75", "q:0.9"])
    );
    let summary = &b["reports"][0]["groups"][0]["summary"];
    assert!(summary["mean"].is_null());
    assert_eq!(summary["quantiles"][2]["convention"], "midpoint");
    assert!(chosen.text().contains("10%") && chosen.text().contains("90%"));
    let moments = json(
        &p.run(&Options::new().report_summary(0, ReportSummary::moments()))
            .unwrap(),
    );
    assert_eq!(
        moments["reports"][0]["groups"][0]["summary"]["mean"],
        a["reports"][0]["groups"][0]["summary"]["mean"]
    );
    assert_eq!(
        moments["reports"][0]["groups"][0]["summary"]["quantiles"],
        serde_json::json!([])
    );
}

#[cfg(feature = "json")]
#[test]
fn analytic_and_atomic_quantiles_keep_their_conventions_and_uncertainty() {
    for source in [
        "report one_of([0:50%,10:50%])",
        "let side~d2; let x~if side==1 {uniform(0,1)} else {uniform(10,11)}; report x",
    ] {
        let out = probl::compile("conventions", source)
            .unwrap()
            .run(&Options::new().report_summary(0, "q:0.5,median".parse().unwrap()))
            .unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&out.reports_json(&probl::ReportJsonLimits::default()).unwrap()).unwrap();
        let q = &doc["reports"][0]["groups"][0]["summary"]["quantiles"];
        assert_eq!(q[0]["convention"], "inverse_cdf");
        assert_eq!(q[1]["convention"], "midpoint");
        assert!(q[0]["estimate"]["point"].as_f64().unwrap() < q[1]["estimate"]["point"].as_f64().unwrap());
    }
}
