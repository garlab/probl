use probl::{ErrorKind, Limits, Options, ReportCollection, ReportSummary, SummaryError};

fn options(threads: usize) -> Options {
    let mut limits = Limits::default();
    limits.max_threads = threads;
    Options::new().runs(2100).seed(42).limits(limits)
}

#[test]
fn weighted_repeated_visits_keep_moments_errors_and_model_draws() {
    let p = probl::compile(
        "moments",
        r#"
let x ~ d6
score if x > 3 { 90% } else { 10% }
for k in 1..x {
  report x + k by k mod 2 as "numbers"
  report x > 3 by k mod 2 as "facts"
}
report x as "untouched"
"#,
    )
    .unwrap();
    let exact = p.run(&options(1).report_summary(0, ReportSummary::moments())).unwrap();
    let collect = |threads| {
        options(threads)
            .report_collection(0, ReportCollection::Moments)
            .report_collection(1, ReportCollection::Moments)
    };
    let small = p.run(&collect(1)).unwrap();
    let parallel = p.run(&collect(4)).unwrap();
    assert_eq!(small.text(), parallel.text());
    assert_eq!(exact.render(1..3), small.render(1..3));
    assert_eq!(small.reports()[0].collection(), ReportCollection::Moments);
    assert_eq!(small.reports()[2].collection(), ReportCollection::Exact);
    for (a, b) in exact.reports()[0].groups().iter().zip(small.reports()[0].groups()) {
        assert!(a.distribution().is_some());
        assert!(b.distribution().is_none());
        let (a, b) = (a.numeric().unwrap(), b.numeric().unwrap());
        for (x, y) in [
            (a.mean().unwrap(), b.mean().unwrap()),
            (a.sd().unwrap(), b.sd().unwrap()),
        ] {
            assert!((x.point().unwrap() - y.point().unwrap()).abs() < 1e-12);
            assert_eq!(format!("{:?}", x.sampling()), format!("{:?}", y.sampling()));
        }
        assert_eq!(b.quantile(0.5).unwrap_err(), SummaryError::Unavailable);
        assert_eq!(b.quantile(f64::NAN).unwrap_err(), SummaryError::InvalidQuantile);
    }
    #[cfg(feature = "json")]
    {
        let json = |o: &probl::Outcome| {
            serde_json::from_str::<serde_json::Value>(&o.reports_json(&Default::default()).unwrap()).unwrap()
        };
        let (a, b, c) = (json(&exact), json(&small), json(&parallel));
        assert_eq!(b, c);
        assert_eq!(a["run"], b["run"]);
        assert_eq!(a["reports"][1]["groups"], b["reports"][1]["groups"]);
        assert_eq!(a["reports"][2], b["reports"][2]);
        assert_eq!(b["reports"][0]["collection"], "moments");
        assert_eq!(
            b["reports"][0]["groups"][0]["summary"]["quantile_method"],
            "unavailable"
        );
        assert_eq!(
            b["reports"][0]["groups"][0]["summary"]["quantiles"],
            serde_json::json!([])
        );
    }
}

#[test]
fn invalid_configuration_precedes_execution() {
    let p = probl::compile("validation", "print(\"side effect\"); report 1").unwrap();
    for o in [
        Options::new().report_collection(0, ReportCollection::Moments),
        options(1).report_collection(1, ReportCollection::Moments),
        options(1)
            .report_collection(0, ReportCollection::Moments)
            .report_summary(0, ReportSummary::forecast()),
    ] {
        let mut printed = Vec::new();
        let e = p.run_with(&o, &mut |s| printed.push(s.to_owned())).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::Usage);
        assert!(printed.is_empty());
    }
}

#[test]
fn incompatible_values_fail_instead_of_being_discarded() {
    for source in [
        "report \"cat\"",
        "report date(2026,1,1)",
        "report [1,2]",
        "report 9007199254740993",
        "report d6",
        "let x~d2; report if x==1 {true} else {1}",
    ] {
        let p = probl::compile("types", source).unwrap();
        let e = p
            .run(&options(1).report_collection(0, ReportCollection::Moments))
            .unwrap_err();
        assert!(e.to_string().contains("moments collection"), "{source}: {e}");
    }
}

#[test]
fn group_limits_apply_within_and_across_batches() {
    for (source, cap) in [
        ("for k in 0..3 {report k by k}", 3),
        ("let k~uniform(0,1); report 1 by k", 1500),
    ] {
        let p = probl::compile("groups", source).unwrap();
        for threads in [1, 4] {
            let mut limits = Limits::default();
            limits.max_report_groups = cap;
            limits.max_threads = threads;
            let e = p
                .run(
                    &options(threads)
                        .limits(limits)
                        .report_collection(0, ReportCollection::Moments),
                )
                .unwrap_err();
            assert_eq!(e.kind(), ErrorKind::Limit);
            assert!(e.to_string().contains("max_report_groups"));
        }
    }
}

#[test]
fn percentage_values_partial_runs_and_unreached_reports() {
    let p = probl::compile("partial", "let x~d2; report prob(0.5/(x-1)); if false {report 1}").unwrap();
    let exact = p.run(&options(1)).unwrap_err();
    let small = p
        .run(
            &options(1)
                .report_collection(0, ReportCollection::Moments)
                .report_collection(1, ReportCollection::Moments),
        )
        .unwrap_err();
    let (a, b) = (exact.partial().unwrap(), small.partial().unwrap());
    assert!(b.text().contains("50.00%"));
    assert!(b.reports()[1].groups().is_empty());
    assert_eq!(
        a.reports()[0].groups()[0].numeric().unwrap().mean(),
        b.reports()[0].groups()[0].numeric().unwrap().mean()
    );
}

#[test]
fn cancellation_still_stops_moments_runs() {
    let p = probl::compile("cancel", "let x~d6; report x").unwrap();
    let cancel = probl::Cancel::new();
    let progress_cancel = cancel.clone();
    let error = p
        .run(
            &options(1)
                .runs(10000)
                .cancel(&cancel)
                .progress(move |_, _| progress_cancel.cancel())
                .report_collection(0, ReportCollection::Moments),
        )
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Limit);
}
