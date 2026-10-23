use probl::{ErrorKind, Limits, Options, ReportCollection};

fn options(bytes: usize, runs: u64, threads: usize) -> Options {
    let mut limits = Limits::default();
    limits.max_report_summary_bytes = bytes;
    limits.max_threads = threads;
    Options::new().runs(runs).seed(42).limits(limits)
}

#[test]
fn sites_share_one_budget_including_shared_large_keys() {
    let key = "x".repeat(200_000);
    let p = probl::compile(
        "memory",
        &format!("let key = \"{key}\"; report 1 by key; report 2 by key"),
    )
    .unwrap();
    let base = options(300_000, 1, 1);
    for id in 0..2 {
        p.run(&base.clone().report_collection(id, ReportCollection::Moments))
            .unwrap();
    }
    let error = p
        .run(
            &base
                .report_collection(0, ReportCollection::Moments)
                .report_collection(1, ReportCollection::Moments),
        )
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Limit);
    assert!(error.to_string().contains("max_report_summary_bytes"));
    assert!(
        error.partial().is_none(),
        "collector limits must not silently drop trials"
    );
}

#[test]
fn active_batches_are_bounded_and_freed_between_runs() {
    let p = probl::compile("memory", "let x~uniform(0,1); report x by 0; report x > 0.5").unwrap();
    for threads in [1, 4] {
        let collect = |bytes, runs| {
            options(bytes, runs, threads)
                .report_collection(0, ReportCollection::Moments)
                .report_collection(1, ReportCollection::Moments)
        };
        // A few completed groups fit; the full active batch does not.
        p.run(&collect(100_000, 2)).unwrap();
        let e = p.run(&collect(100_000, 4000)).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::Limit);
        assert!(e.to_string().contains("max_report_summary_bytes"));
        // A new execution owns a fresh budget, even after an error.
        p.run(&collect(100_000, 2)).unwrap();
        let a = p.run(&collect(4 * 1024 * 1024, 4000)).unwrap();
        let b = p.run(&collect(16 * 1024 * 1024, 4000)).unwrap();
        assert_eq!(a.text(), b.text());
    }
    // Twenty batches fit in a live allowance much smaller than cumulative use.
    p.run(
        &options(1024 * 1024, 20_000, 1)
            .report_collection(0, ReportCollection::Moments)
            .report_collection(1, ReportCollection::Moments),
    )
    .unwrap();
}

#[test]
fn exact_and_unreached_reports_do_not_spend_summary_budget() {
    let exact = probl::compile("exact", "let x~d6; report x").unwrap();
    exact.run(&options(0, 100, 1)).unwrap();
    let e = exact
        .run(&options(0, 100, 1).report_collection(0, ReportCollection::Moments))
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Limit);
    let absent = probl::compile("absent", "if false {report 1}").unwrap();
    absent
        .run(&options(0, 100, 1).report_collection(0, ReportCollection::Moments))
        .unwrap();
}

#[test]
fn plain_nested_keys_work_and_opaque_keys_are_diagnosed() {
    for key in ["[1, [2, 3], \"hello\"]", "{x: 1, y: \"key\"}", "2^100", "1..10"] {
        let p = probl::compile("keys", &format!("report 1 by {key}")).unwrap();
        p.run(&options(1024 * 1024, 10, 1).report_collection(0, ReportCollection::Moments))
            .unwrap();
    }
    for source in ["report 1 by [d6]", "fn f(x) {x}; report 1 by f"] {
        let p = probl::compile("keys", source).unwrap();
        p.run(&options(0, 10, 1)).unwrap();
        let e = p
            .run(&options(1024 * 1024, 10, 1).report_collection(0, ReportCollection::Moments))
            .unwrap_err();
        assert!(e.to_string().contains("bounded report keys"), "{e}");
    }
}
